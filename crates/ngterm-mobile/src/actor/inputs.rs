//! The chat's actions on the actor ([`crate::chat::input`]): each runs as a local task that takes the pane's lead,
//! settles, reads the screen and sends its plan's keys, one action per pane at a time; and the approval card's labels,
//! read from the screen as it stands.

use std::rc::Rc;
use std::sync::Arc;

use ngterm_core::live::lead::LeadRole;
use tokio::time::Instant;

use super::{Command, State};
use crate::api::{PaneInputError, PaneSlot};
use crate::chat::ChatKey;
use crate::chat::input::{
    ACTION_IN_FLIGHT, Action, AgentInputOutcome, InputTiming, NO_AGENT_TO_DRIVE, Next, Plan, approval_labels,
};
use crate::chat::matchers::Agent;
use crate::pane::{PaneRef, Standing};
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
                let _ = reply.send(act(&state, &pane, slot, action).await);
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

/// The pane an action is on: as the core names it, and the slot of the handle that asked for it.
struct On {
    key: ChatKey,
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
    Ok((On { key, slot }, agent))
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
        .map_or(crate::chat::matchers::Screen::Unknown, |terminal| {
            agent.read(&terminal.frame().frame)
        });
    Ok(approval_labels(agent, &screen))
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

/// Runs `action` on the pane: lead, then settle, look and send until its plan is done.
async fn act<T: Transport>(
    state: &Rc<State<T>>,
    reference: &PaneRef,
    slot: Arc<PaneSlot>,
    action: Action,
) -> Result<AgentInputOutcome, PaneInputError> {
    let (on, agent) = target(state, reference, slot)?;
    standing(state, &on).ok_or(PaneInputError::Closed)?;
    let agent = agent.ok_or_else(|| refused(NO_AGENT_TO_DRIVE))?;
    let _acting = Acting::take(state, &on.key)?;
    let timing = state.wiring.timing;
    let Some(attach) = lead(state, &on, timing).await? else {
        return Ok(AgentInputOutcome::LeadRefused);
    };
    let mut plan = Plan::new(action);
    let mut busy = 0;
    loop {
        let step = match settle(state, &on, attach, timing).await {
            Ok(true) => look(state, &on, attach, agent, &mut plan),
            Ok(false) => Ok(Looked::Done(plan.stopped(AgentInputOutcome::LeadRefused))),
            Err(error) => Err(error),
        };
        match step {
            Ok(Looked::Done(outcome)) => return Ok(outcome),
            Ok(Looked::Again) => busy = 0,
            Ok(Looked::Busy) => {
                busy += 1;
                if busy >= BUSY_LOOKS {
                    return Ok(plan.stopped(AgentInputOutcome::NotOnScreen));
                }
            }
            // hidden after keys went: they went
            Err(_) if plan.sent() => return Ok(AgentInputOutcome::Unconfirmed),
            Err(error) => return Err(error),
        }
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
    /// The frame could not be read now: settle and look again.
    Busy,
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

/// One look at the screen and what the plan makes of it, sent at once into the terminal read. Nothing is awaited
/// here, so no output, role report or attach is handled between the read and the send but what the terminal took on
/// its own threads.
fn look<T: Transport>(
    state: &State<T>,
    on: &On,
    attach: u64,
    agent: Agent,
    plan: &mut Plan,
) -> Result<Looked, PaneInputError> {
    drain(state, &on.key);
    let now = standing(state, on).ok_or(PaneInputError::Closed)?;
    let Some(terminal) = now.terminal.as_ref().filter(|_| leads(&now, attach)) else {
        return Ok(Looked::Done(plan.stopped(AgentInputOutcome::LeadRefused)));
    };
    let snapshot = terminal.frame();
    // a frame made while the program holds its output (mode 2026) is the screen before its update
    if snapshot.stale || snapshot.frame.held {
        return Ok(Looked::Busy);
    }
    match plan.next(&agent.read(&snapshot.frame)) {
        Next::Done(outcome) => Ok(Looked::Done(outcome)),
        Next::Send { keys, look } => {
            for (at, key) in keys.iter().enumerate() {
                if let Err(error) = key.send(terminal) {
                    // a key refused after others went leaves the screen unconfirmed; the first one, nothing sent
                    return if plan.sent() || at > 0 {
                        Ok(Looked::Done(AgentInputOutcome::Unconfirmed))
                    } else {
                        Err(error)
                    };
                }
            }
            plan.mark_sent();
            Ok(if look {
                Looked::Again
            } else {
                Looked::Done(AgentInputOutcome::Sent)
            })
        }
    }
}
