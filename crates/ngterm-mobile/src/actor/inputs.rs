//! The chat's actions on the actor ([`crate::chat::input`]): each runs as a local task that takes the pane's lead,
//! settles, reads the screen and sends its plan's keys, one action per pane at a time; and the approval card's labels,
//! read from the screen as it stands. The screen is always the program's own ([`readable`]), never what the terminal
//! view shows of it.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use ngterm_core::live::lead::LeadRole;
use ngterm_term::ActiveScreen;
use tokio::time::Instant;

use super::{Command, State};
use crate::api::{PaneInputError, PaneSlot};
use crate::applog::tag;
use crate::chat::input::{
    ACTION_IN_FLIGHT, Action, AgentInputOutcome, InputTiming, NO_AGENT_TO_DRIVE, Next, Plan, approval_labels,
};
use crate::chat::matchers::Agent;
use crate::chat::{ANSWERED_DEADLINE, ChatKey, Notice};
use crate::note;
use crate::pane::{PaneRef, Standing};
use crate::sessions;
use crate::transport::Transport;

/// Handles a chat action command.
pub(super) fn command<T: Transport>(state: &Rc<State<T>>, command: Command) {
    match command {
        Command::PaneInput {
            pane,
            slot,
            action,
            reply,
        } => {
            let state = Rc::clone(state);
            tokio::task::spawn_local(async move {
                // A caller that stopped waiting has nothing to be told; the action still ran its course.
                let _ = reply.send(noted(&state, &pane, slot, action).await);
            });
        }
        Command::ApprovalLabels { pane, slot, reply } => {
            let _ = reply.send(labels(state, &pane, slot));
        }
        // The loop handles every other command itself.
        _ => {}
    }
}

fn refused(reason: &str) -> PaneInputError {
    PaneInputError::Refused {
        reason: reason.to_owned(),
    }
}

/// The pane an action is on: as the core names it, as the handle names it, and the slot of the handle that asked for it.
struct On {
    key: ChatKey,
    pane: PaneRef,
    slot: Arc<PaneSlot>,
}

/// The pane `reference` names for the handle whose slot is `slot`, and the agent its origin announced in it.
fn target<T: Transport>(
    state: &State<T>,
    reference: &PaneRef,
    slot: Arc<PaneSlot>,
) -> Result<(On, Option<Agent>), PaneInputError> {
    let key = ChatKey::of(reference).map_err(|_| PaneInputError::Closed)?;
    let agent = state.chats.borrow().agent(&key).and_then(Agent::of);
    Ok((
        On {
            key,
            pane: reference.clone(),
            slot,
        },
        agent,
    ))
}

/// Where the pane stands, while it is the one the handle was given: a pane hidden since (and maybe shown again, with
/// another slot) is `None`, as a hidden one is.
fn standing<T: Transport>(state: &State<T>, on: &On) -> Option<Standing> {
    let key = &on.key;
    state
        .panes
        .borrow()
        .standing(&key.machine_id, &key.session_id, key.pane)
        .filter(|standing| Arc::ptr_eq(&standing.slot, &on.slot))
}

/// The card's labels: the dialog's on screen, or the agent's usual ones.
fn labels<T: Transport>(
    state: &State<T>,
    reference: &PaneRef,
    slot: Arc<PaneSlot>,
) -> Result<Vec<String>, PaneInputError> {
    let (on, agent) = target(state, reference, slot)?;
    let standing = standing(state, &on).ok_or(PaneInputError::Closed)?;
    let Some(agent) = agent else {
        return Ok(Vec::new());
    };
    let screen = standing
        .terminal
        .and_then(|terminal| readable(&terminal))
        .map_or(crate::chat::matchers::Screen::Unknown, |screen| {
            agent.read(&screen.frame)
        });
    Ok(approval_labels(agent, &screen))
}

/// The program's screen in `terminal` (its active area, with the generation it was read at), when it is one the
/// program drew for the terminal's grid: `None` while the program holds its output (synchronized output; a hold past
/// its timeout is ended first, as a paint ends it), while the screen is the terminal's reflow after a new column or
/// row count the program has not drawn since, or when it could not be read. The same grid again or a new cell size
/// alone reflows nothing and leaves an idle screen readable. A grid the view gives that the terminal has not taken
/// yet is no reason to wait: the screen read is still the terminal's and the program's, and a resize landing after
/// the read voids the send ([`crate::chat::input::Key::send_if`]); one the terminal refused never lands, and waiting
/// for it would leave the pane busy for good. What the terminal view shows plays no part: the view scrolled back
/// (onto an old copy of a dialog, say), a selection, a search's highlights and whatever the app draws over the view
/// leave the active area as it is, and change no generation.
fn readable(terminal: &ngterm_term::Pane) -> Option<ActiveScreen> {
    terminal
        .active_screen()
        .ok()
        .flatten()
        .filter(|screen| !screen.reflowed)
}

/// Holds a pane for one action; dropping it lets the next one in.
struct Acting<T: Transport> {
    state: Rc<State<T>>,
    key: ChatKey,
}

impl<T: Transport> Acting<T> {
    fn take(state: &Rc<State<T>>, key: &ChatKey) -> Result<Self, PaneInputError> {
        if !state.acting.borrow_mut().insert(key.clone()) {
            return Err(refused(ACTION_IN_FLIGHT));
        }
        Ok(Self {
            state: Rc::clone(state),
            key: key.clone(),
        })
    }
}

impl<T: Transport> Drop for Acting<T> {
    fn drop(&mut self) {
        self.state.acting.borrow_mut().remove(&self.key);
    }
}

/// Runs `action` ([`act`]) and logs how it ended and the last screen it read: why a message was sent, held or refused,
/// never the message.
async fn noted<T: Transport>(
    state: &Rc<State<T>>,
    reference: &PaneRef,
    slot: Arc<PaneSlot>,
    action: Action,
) -> Result<AgentInputOutcome, PaneInputError> {
    let kind = action.kind();
    let seen = Cell::new("not-read");
    let outcome = act(state, reference, slot, action, &seen).await;
    let (ended, why) = match &outcome {
        Ok(outcome) => (outcome.class(), None),
        Err(error) => (error.class(), error.why()),
    };
    note!(
        INFO,
        tag::CHAT,
        "action",
        action = kind,
        outcome = ended,
        why = why,
        screen = seen.get(),
        session = reference.session_id.as_str(),
        pane = reference.pane_id.as_str()
    );
    outcome
}

/// Runs `action` on the pane: lead, then settle, look and send until its plan is done; after keys went, settle and
/// look until the screen shows what they did or [`InputTiming::confirm`] has gone by.
async fn act<T: Transport>(
    state: &Rc<State<T>>,
    reference: &PaneRef,
    slot: Arc<PaneSlot>,
    action: Action,
    seen: &Cell<&'static str>,
) -> Result<AgentInputOutcome, PaneInputError> {
    let (on, agent) = target(state, reference, slot)?;
    standing(state, &on).ok_or(PaneInputError::Closed)?;
    let agent = agent.ok_or_else(|| refused(NO_AGENT_TO_DRIVE))?;
    let _acting = Acting::take(state, &on.key)?;
    let notice = notice(state, &on, &action);
    let approval = matches!(action, Action::Approval(..));
    let can_answer = !approval || notice.is_some();
    let timing = state.wiring.timing;
    let Some(attach) = lead(state, &on, timing).await? else {
        return Ok(AgentInputOutcome::LeadRefused);
    };
    let mut plan = Plan::new(action, agent);
    let mut sent: Option<Went> = None;
    let mut busy = 0;
    let outcome = loop {
        let step = match settle(state, &on, attach, timing).await {
            Ok(true) => look(state, &on, attach, agent, (&mut plan, &mut sent, can_answer), seen),
            Ok(false) => Ok(Looked::Done(plan.stopped(AgentInputOutcome::LeadRefused))),
            Err(error) => Err(error),
        };
        match step {
            Ok(Looked::Done(outcome)) => {
                break Ok(outcome);
            }
            Ok(Looked::Again) => busy = 0,
            Ok(Looked::Pending) => {
                busy = 0;
                if sent.is_none_or(|went| went.at.elapsed() >= timing.confirm) {
                    break Ok(plan.stopped(AgentInputOutcome::NotOnScreen));
                }
            }
            Ok(Looked::Busy) => {
                busy += 1;
                if busy >= BUSY_LOOKS {
                    break Ok(plan.stopped(AgentInputOutcome::NotOnScreen));
                }
            }
            // hidden after keys went: they went
            Err(_) if plan.sent() => break Ok(AgentInputOutcome::Unconfirmed),
            Err(error) => break Err(error),
        }
    };
    let working = if approval {
        plan.answered().map(|(_, working)| working)
    } else {
        Some(true)
    };
    if plan.sent()
        && let Some(working) = working
        && let Some(notice) = notice
    {
        let command = state.chats.borrow().notice_command(&on.key, &notice, working);
        if !matches!(sessions::run(&*notice.connection, &command, ANSWERED_DEADLINE).await, Ok(result) if result.status == 0)
        {
            return Ok(AgentInputOutcome::Unconfirmed);
        }
    }
    outcome
}

fn notice<T: Transport>(state: &State<T>, on: &On, action: &Action) -> Option<Notice<T::Connection>> {
    let chats = state.chats.borrow();
    match action {
        Action::Approval(card, _) => chats
            .prompt(&on.key, card)
            .and_then(|prompt| chats.answer_notice(&on.key, prompt)),
        Action::Interrupt => {
            let connection = Rc::clone(&state.connections.borrow().get(&on.key.machine_id)?.connection);
            chats.interrupt_notice(&on.key, connection)
        }
        _ => None,
    }
}

/// The looks in a row that may find the terminal busy (its frame stale, or held by the program) before an action
/// gives up on reading the screen.
const BUSY_LOOKS: u32 = 10;

/// What a look made of the screen.
enum Looked {
    Done(AgentInputOutcome),
    /// Keys went: settle and look again.
    Again,
    /// The screen does not show yet what the last keys did: settle and look again, until [`InputTiming::confirm`].
    Pending,
    /// The frame could not be read now: settle and look again.
    Busy,
}

/// The last keys that went: the screen generation they were sent to, and when.
#[derive(Debug, Clone, Copy)]
struct Went {
    generation: u64,
    at: Instant,
}

/// Whether `standing` is still the attach `attach`, leading.
fn leads(standing: &Standing, attach: u64) -> bool {
    standing.key == attach && standing.live && standing.role == Some(LeadRole::Leader)
}

/// Takes the pane's lead, unless it holds it: the attach that leads, or `None` when the lead was not had (the claim
/// reported another role, its attach was replaced, nothing came in time, or the pane can't claim).
async fn lead<T: Transport>(state: &Rc<State<T>>, on: &On, timing: InputTiming) -> Result<Option<u64>, PaneInputError> {
    let key = &on.key;
    drain(state, key);
    let now = standing(state, on).ok_or(PaneInputError::Closed)?;
    if leads(&now, now.key) {
        return Ok(Some(now.key));
    }
    if now.ended {
        return Ok(None);
    }
    if !now.claiming {
        let events = state
            .panes
            .borrow_mut()
            .take_lead(&key.machine_id, &key.session_id, key.pane);
        state.emit_all(events);
    }
    let claim = standing(state, on).ok_or(PaneInputError::Closed)?;
    if !claim.claiming {
        return Ok(None);
    }
    let deadline = Instant::now() + timing.lead_timeout;
    while Instant::now() < deadline {
        tokio::time::sleep(timing.poll).await;
        drain(state, key);
        let now = standing(state, on).ok_or(PaneInputError::Closed)?;
        if now.key != claim.key {
            return Ok(None);
        }
        match now.role {
            Some(LeadRole::Leader) if now.live => return Ok(Some(now.key)),
            Some(LeadRole::Leader) | None => {}
            Some(LeadRole::Follower | LeadRole::Unowned) => return Ok(None),
        }
    }
    Ok(None)
}

/// The terminal's pending news, handled now: its role reports applied.
fn drain<T: Transport>(state: &State<T>, key: &ChatKey) {
    let events = state
        .panes
        .borrow_mut()
        .drain(&key.machine_id, &key.session_id, key.pane);
    state.emit_all(events);
}

/// What a settle compares: the bytes the terminal took, its frame and the view's grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mark {
    bytes: u64,
    frame: Option<u64>,
    size: (u16, u16),
}

/// Waits until the attach's screen settles: nothing new for [`InputTiming::quiet`], or [`InputTiming::settle_max`]
/// gone by. Whether the attach still leads.
async fn settle<T: Transport>(
    state: &Rc<State<T>>,
    on: &On,
    attach: u64,
    timing: InputTiming,
) -> Result<bool, PaneInputError> {
    let started = Instant::now();
    let mut last: Option<Mark> = None;
    let mut quiet_since = started;
    loop {
        tokio::time::sleep(timing.poll).await;
        let now = standing(state, on).ok_or(PaneInputError::Closed)?;
        let Some(terminal) = now.terminal.as_ref().filter(|_| leads(&now, attach)) else {
            return Ok(false);
        };
        let snapshot = terminal.frame();
        let mark = Mark {
            bytes: terminal.stats().bytes,
            frame: (!snapshot.stale && !snapshot.frame.held).then_some(snapshot.frame.seq),
            size: (now.size.cols, now.size.rows),
        };
        let at = Instant::now();
        if last != Some(mark) || mark.frame.is_none() {
            last = Some(mark);
            quiet_since = at;
        } else if at - quiet_since >= timing.quiet {
            return Ok(true);
        }
        if at - started >= timing.settle_max {
            return Ok(true);
        }
    }
}

/// One look at the program's screen ([`readable`]) and what the plan makes of it, sent at once into the terminal
/// read. Nothing is awaited here, so no role report or attach is handled between the read and the send; output the
/// terminal took on its own threads meanwhile (or a resize) makes the send a no-op
/// ([`crate::chat::input::Key::send_if`]), the plan is put back as it was, and the screen is settled and read again.
/// A screen that can't be read now is busy. The screen the last keys were sent to, with no output since, is pending:
/// what they did has not come back yet.
fn look<T: Transport>(
    state: &State<T>,
    on: &On,
    attach: u64,
    agent: Agent,
    (plan, sent, can_answer): (&mut Plan, &mut Option<Went>, bool),
    seen: &Cell<&'static str>,
) -> Result<Looked, PaneInputError> {
    drain(state, &on.key);
    let now = standing(state, on).ok_or(PaneInputError::Closed)?;
    let Some(terminal) = now.terminal.as_ref().filter(|_| leads(&now, attach)) else {
        return Ok(Looked::Done(plan.stopped(AgentInputOutcome::LeadRefused)));
    };
    let Some(screen) = readable(terminal) else {
        seen.set("unreadable");
        return Ok(Looked::Busy);
    };
    let generation = screen.generation;
    if sent.is_some_and(|went| went.generation == generation) {
        return Ok(Looked::Pending);
    }
    let before = plan.clone();
    let read = agent.read(&screen.frame);
    seen.set(read.class());
    match plan.next(&read) {
        Next::Done(outcome) => Ok(Looked::Done(outcome)),
        Next::Wait => Ok(Looked::Pending),
        Next::Send { keys, look } => {
            if !can_answer {
                return Err(refused(
                    "The approval is no longer connected to its chat. Reopen the chat before answering.",
                ));
            }
            for (at, key) in keys.iter().enumerate() {
                match key.send_if(terminal, generation) {
                    // the user answered on the phone, as if typing there
                    Ok(true) => {
                        state.presentations.borrow_mut().typed(&on.pane, key.badge_kind());
                    }
                    // the screen changed after the read: nothing of this step went, so it is weighed again
                    Ok(false) if at == 0 => {
                        *plan = before;
                        return Ok(Looked::Busy);
                    }
                    Ok(false) => return Ok(Looked::Done(AgentInputOutcome::Unconfirmed)),
                    // a key refused after others went leaves the screen unconfirmed; the first one, nothing sent
                    Err(_) if plan.sent() || at > 0 => return Ok(Looked::Done(AgentInputOutcome::Unconfirmed)),
                    Err(error) => return Err(error),
                }
            }
            plan.mark_sent();
            *sent = Some(Went {
                generation,
                at: Instant::now(),
            });
            Ok(if look {
                Looked::Again
            } else {
                Looked::Done(AgentInputOutcome::Sent)
            })
        }
    }
}
