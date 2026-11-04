//! Shown panes: each one an `ngterm-session attach` of the pane's daemon on an exec channel with a pty, feeding an
//! `ngterm-term` stream pane (`Pane::stream`) the app draws and types into ([`crate::api::Pane`]).
//!
//! - **Threads.** Once the channel opens, a feeder thread reads it into the pane's `StreamFeed` (which blocks while
//!   the terminal is behind, holding the channel back) and a writer thread sends what the pane's `StreamOut` gives
//!   (input, the terminal's replies, resizes) down the channel. Both report back to the actor as `Note`s, as the
//!   pane's wake does; a note of an attach since replaced is dropped. A stopped writer sends what is still queued (a
//!   denied clipboard read's reply) and closes the channel once the queue has been quiet for [`WRITER_POLL`].
//! - **Leadership.** The attach tail carries a `LeadAttachment`: the first attach and every automatic one claim
//!   nothing (they lead only a session nobody leads), [`crate::MobileCore::take_lead`] claims, as does a chat action
//!   on a pane that does not lead ([`crate::chat::input`], which weighs [`Panes::standing`]). The attach client's
//!   role reports arrive as titles under `LeadNotice::PREFIX`: filtered through the pane's `LeadState` into
//!   [`MobileEvent::PaneLead`], never shown as a title. A follower is covered; an unowned pane attaches again.
//! - **How an attach ends** ([`PaneEnd`]): a *lost* link covers the pane, and it attaches again into a fresh terminal
//!   once its machine is connected (at once on reconnect; on the reconnect backoff when only the channel went);
//!   an *exit* reports the program's status after `end_of_output` and `exited`, and attaches again, attach-only, when
//!   the origin reports the pane's shell respawned (`RemoteRespawnBook`, fed by the session's presentation stream,
//!   whose latest counts are kept for a pane shown later);
//!   an exit 1 after the gone guard's [`SESSION_GONE`] line is checked against a fresh listing, and a session no
//!   longer listed is *gone*: final.
//! - **Clipboard reads** (OSC 52) stop the pane's output until answered: each raises
//!   [`MobileEvent::ClipboardReadRequest`] under an id of the core's own, answered through
//!   [`crate::MobileCore::answer_clipboard_read`]. Hiding the pane, losing its link or its program's exit deny any read
//!   still open and dismiss it ([`MobileEvent::ClipboardReadDismiss`]).
//! - **A pane shown with no grid** (0 columns or rows) waits without a terminal and attaches once the view gives it
//!   one ([`crate::api::Pane::resize`]).
//! - **Kitty images** are off (`kitty_image_limit` 0): the attach client's probe reports no support.

mod records;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use ngterm_core::live::lead::{LeadAttachment, LeadNotice, LeadRole, LeadState};
use ngterm_core::pane_env::uuid_string;
use ngterm_core::remote::frames::PresentationRespawns;
use ngterm_core::remote::reconnect::retry_delay;
use ngterm_core::remote::respawn::{Mark, RemoteRespawnBook};
use ngterm_core::remote::ssh::{SESSION_GONE, attach_tail};
use ngterm_proto::ControlLiveEndpoint;
use ngterm_term::{
    ClipboardAnswer, ClipboardReadRequest, ClipboardWrite, GridSize, Outgoing, PaneConfig, PaneEvent, ReadScope,
    StreamFeed, StreamOut, Wake,
};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use uuid::Uuid;

pub use self::records::{ClipboardReply, PaneClipboard, PaneEnd, PaneRef, PaneRole};
use crate::api::{MobileError, MobileEvent, PaneSlot};
use crate::presentation::Respawns;
use crate::transport::{Channel, ChannelEnd, ChannelOutput, ChannelReader, ChannelWriter, Connection, TransportError};

/// How long a pane's writer waits for something to send before it looks whether it was stopped; stopped, it closes
/// the channel once a wait passes with nothing sent.
pub const WRITER_POLL: Duration = Duration::from_millis(100);

/// The most waits a stopped writer makes before it closes the channel whatever is still queued.
const STOPPED_POLLS: u32 = 10;

/// The most bytes a pane's writer leaves queued for ssh before it stops taking the terminal's output: past it the
/// input waits in the pane's own queue, whose limits push back on typing and pasting and drop surplus replies.
pub const WRITE_BACKLOG: usize = 64 * 1024;

/// Why a pane was refused that its session doesn't hold.
pub const UNKNOWN_PANE: &str = "the session has no such pane";

/// What a pane's threads and tasks tell the actor, tagged with the attach they belong to.
#[derive(Debug)]
pub(crate) enum Note {
    /// The terminal has news: a frame to draw, events to drain (its wake).
    News(u64),
    /// The attach's channel opened, or failed to.
    Opened(u64, Result<Channel, TransportError>),
    /// The attach's channel ended; `gone` when its output ended with the gone guard's line.
    Ended { key: u64, end: ChannelEnd, gone: bool },
    /// A lost attach's wait before it attaches again is over.
    Retry(u64),
    /// The view gave a grid to the shown pane with this id while it held no terminal.
    Sized(u64),
}

/// Where the notes go: the actor's.
pub(crate) type Notes = mpsc::UnboundedSender<Note>;

/// A pane the app asks to show, resolved from its machine's listing.
#[derive(Debug, Clone)]
pub(crate) struct Target {
    pub(crate) machine_id: String,
    pub(crate) session_id: String,
    pub(crate) pane: Uuid,
    pub(crate) daemon: String,
    pub(crate) endpoint: ControlLiveEndpoint,
}

/// A session listing that must be read before a pane's exit can be called its session's end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GoneCheck {
    pub(crate) key: u64,
    pub(crate) machine_id: String,
    pub(crate) session_id: String,
}

/// Where a shown pane's attach stands ([`Panes::standing`]).
pub(crate) struct Standing {
    /// The current attach's key: another attach replaced the one an action weighed when it differs.
    pub(crate) key: u64,
    /// Its role as its attach last reported it.
    pub(crate) role: Option<LeadRole>,
    /// Its attach claims the lead and has not reported yet.
    pub(crate) claiming: bool,
    /// Its program exited, or its session is gone.
    pub(crate) ended: bool,
    /// Its channel is open.
    pub(crate) live: bool,
    pub(crate) terminal: Option<Arc<ngterm_term::Pane>>,
    /// The grid the view gave.
    pub(crate) size: GridSize,
    /// The slot the pane's handles share while it stays shown: a pane hidden and shown again has another.
    pub(crate) slot: Arc<PaneSlot>,
}

/// What handling a note made: events, and a listing the actor is to read ([`Panes::gone_checked`]).
#[derive(Debug, Default)]
pub(crate) struct Handled {
    pub(crate) events: Vec<MobileEvent>,
    pub(crate) check: Option<GoneCheck>,
}

impl From<Vec<MobileEvent>> for Handled {
    fn from(events: Vec<MobileEvent>) -> Self {
        Self { events, check: None }
    }
}

/// The terminal a shown pane's channel feeds: Kitty images off, its wake noting news for the actor.
pub(crate) fn config(size: GridSize, wake: Wake) -> PaneConfig {
    let mut config = PaneConfig::new(size);
    config.kitty_image_limit = 0;
    config.wake = Some(wake);
    config
}

/// Where a shown pane's attach is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Its channel is opening.
    Opening,
    /// Its channel is open.
    Live,
    /// Its link was lost (or never opened): it attaches again once it can.
    Waiting,
    /// Its program exited after the gone guard's line; a listing decides.
    Checking,
    /// Its program exited: the terminal holds the last screen.
    Exited,
    /// Its session is gone from the machine: final.
    Gone,
}

/// One attach's channel and threads; dropping it stops them (the writer closes the channel).
struct Attachment<C> {
    /// The connection its channel runs on.
    connection: Option<Rc<C>>,
    stop: Arc<AtomicBool>,
    /// The terminal's ends, until the channel opens.
    ends: Option<(StreamFeed, StreamOut)>,
    opening: Option<AbortHandle>,
}

impl<C> Drop for Attachment<C> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(opening) = self.opening.take() {
            opening.abort();
        }
    }
}

/// A clipboard read waiting for the user: the core's id, and the terminal's.
#[derive(Debug, Clone, Copy)]
struct Read {
    id: u64,
    terminal: u64,
}

/// One shown pane.
struct Shown<C> {
    /// Names the pane as long as it is shown, across its attaches.
    id: u64,
    /// Keys this pane's respawn entry: one per shown pane, so two machines showing one origin pane each reattach on
    /// their own.
    local: Uuid,
    reference: PaneRef,
    session: Uuid,
    pane: Uuid,
    daemon: String,
    endpoint: ControlLiveEndpoint,
    slot: Arc<PaneSlot>,
    /// The machine's connection; `None` while it is down.
    connection: Option<Rc<C>>,
    /// The current attach's key.
    key: u64,
    phase: Phase,
    /// The current attach's lead; `None` once its program exited.
    lead: Option<LeadState>,
    attachment: Option<Attachment<C>>,
    reads: Vec<Read>,
    /// Links lost in a row on a connection still up, which set the wait before the next attach.
    losses: u32,
    retry: Option<AbortHandle>,
}

impl<C> Shown<C> {
    fn event(&self, make: impl FnOnce(PaneRef) -> MobileEvent) -> MobileEvent {
        make(self.reference.clone())
    }

    fn lead_event(&self) -> MobileEvent {
        let role = self.lead.as_ref().and_then(LeadState::role).map(PaneRole::from);
        let covered = self.lead.as_ref().is_some_and(LeadState::covered);
        self.event(|pane| MobileEvent::PaneLead { pane, role, covered })
    }

    /// Ends the current attach, if any: its threads stop, its pending retry is off, and the clipboard reads still
    /// open are denied and dismissed.
    fn leave(&mut self) -> Vec<MobileEvent> {
        if let Some(retry) = self.retry.take() {
            retry.abort();
        }
        self.attachment = None;
        let terminal = self.slot.terminal();
        let reads = std::mem::take(&mut self.reads);
        reads
            .into_iter()
            .map(|read| {
                if let Some(terminal) = &terminal {
                    terminal.answer_clipboard_read(read.terminal, ClipboardAnswer::Denied);
                }
                self.event(|pane| MobileEvent::ClipboardReadDismiss { pane, read_id: read.id })
            })
            .collect()
    }

    /// Attaches again after the backoff of one more loss.
    fn retry_later(&mut self, notes: Notes) {
        self.losses += 1;
        let (delay, key) = (retry_delay(self.losses), self.key);
        let retry = tokio::task::spawn_local(async move {
            tokio::time::sleep(delay).await;
            let _ = notes.send(Note::Retry(key));
        });
        self.retry = Some(retry.abort_handle());
    }

    fn is(&self, machine_id: &str, session_id: &str) -> bool {
        self.reference.machine_id == machine_id && self.reference.session_id == session_id
    }
}

/// Every shown pane.
pub(crate) struct Panes<C> {
    shown: Vec<Shown<C>>,
    book: RemoteRespawnBook,
    /// Each attached session's latest respawn counts, by machine and session id, whether or not a pane of it is
    /// shown: a pane shown later takes its baseline from them.
    counts: HashMap<(String, String), PresentationRespawns>,
    notes: Notes,
    /// The last attach key, and the last shown pane's id, taken from one counter.
    last_key: u64,
    last_read: u64,
}

impl<C: Connection> Panes<C> {
    pub(crate) fn new(notes: Notes) -> Self {
        Self {
            shown: Vec::new(),
            book: RemoteRespawnBook::default(),
            counts: HashMap::new(),
            notes,
            last_key: 0,
            last_read: 0,
        }
    }

    /// Shows `target` at `size` on `connection`: a fresh terminal, and its attach. Showing a shown pane hands back its
    /// slot and changes nothing.
    ///
    /// # Errors
    ///
    /// [`MobileError::Remote`] for a session id or pane the attach can't name.
    pub(crate) fn show(
        &mut self,
        target: Target,
        connection: Rc<C>,
        size: GridSize,
    ) -> Result<(Arc<PaneSlot>, Vec<MobileEvent>), MobileError> {
        let unknown = || MobileError::Remote {
            reason: UNKNOWN_PANE.to_owned(),
        };
        let session = Uuid::parse_str(&target.session_id).map_err(|_| unknown())?;
        attach_tail(&target.endpoint, &target.daemon, None).map_err(|_| unknown())?;
        if let Some(shown) = self.find(&target.machine_id, &target.session_id, target.pane) {
            return Ok((Arc::clone(&self.shown[shown].slot), Vec::new()));
        }
        self.last_key += 1;
        let id = self.last_key;
        let notes = self.notes.clone();
        let sized: Wake = Arc::new(move || {
            // An actor that is gone attaches nothing.
            let _ = notes.send(Note::Sized(id));
        });
        let slot = Arc::new(PaneSlot::new(size, sized));
        let local = Uuid::from_u64_pair(0, id);
        self.seed_baseline(&target, local, session);
        self.shown.push(Shown {
            id,
            local,
            reference: PaneRef {
                machine_id: target.machine_id,
                session_id: target.session_id,
                pane_id: uuid_string(target.pane),
            },
            session,
            pane: target.pane,
            daemon: target.daemon,
            endpoint: target.endpoint,
            slot: Arc::clone(&slot),
            connection: Some(connection),
            key: 0,
            phase: Phase::Waiting,
            lead: None,
            attachment: None,
            reads: Vec::new(),
            losses: 0,
            retry: None,
        });
        let events = self.attach(self.shown.len() - 1, LeadAttachment::new(false), false);
        Ok((slot, events))
    }

    /// Starts a newly shown pane's respawn baseline at its session's latest counts, if any were reported: a respawn
    /// missed while the session's stream is down is then found by the next snapshot.
    fn seed_baseline(&mut self, target: &Target, local: Uuid, session: Uuid) {
        let key = (target.machine_id.clone(), target.session_id.clone());
        let Some(counts) = self.counts.get(&key) else {
            return;
        };
        let mark = Mark {
            epoch: counts.epoch.clone(),
            generation: counts.generations.get(&target.daemon).copied().unwrap_or_default(),
        };
        self.book.observe(mark, local, session, true);
    }

    /// Whether the pane `pane` of `session_id` is shown.
    pub(crate) fn is_shown(&self, machine_id: &str, session_id: &str, pane: Uuid) -> bool {
        self.find(machine_id, session_id, pane).is_some()
    }

    /// Where a shown pane's attach stands now, for a chat action ([`crate::chat::input`]); `None` when it is not
    /// shown.
    pub(crate) fn standing(&self, machine_id: &str, session_id: &str, pane: Uuid) -> Option<Standing> {
        let shown = &self.shown[self.find(machine_id, session_id, pane)?];
        let lead = shown.lead.as_ref();
        Some(Standing {
            key: shown.key,
            role: lead.and_then(LeadState::role),
            claiming: lead.is_some_and(|lead| lead.reattaching() && lead.attachment().claim),
            ended: matches!(shown.phase, Phase::Checking | Phase::Exited | Phase::Gone),
            live: shown.phase == Phase::Live,
            terminal: shown.slot.terminal(),
            size: shown.slot.size(),
            slot: Arc::clone(&shown.slot),
        })
    }

    /// Handles a shown pane's pending terminal events now, as its wake would: its role reports are applied before an
    /// action weighs its lead. Nothing is reported when none was pending.
    pub(crate) fn drain(&mut self, machine_id: &str, session_id: &str, pane: Uuid) -> Vec<MobileEvent> {
        let Some(index) = self.find(machine_id, session_id, pane) else {
            return Vec::new();
        };
        let Some(terminal) = self.shown[index].slot.terminal() else {
            return Vec::new();
        };
        let taken = terminal.take_events();
        if taken.is_empty() {
            return Vec::new();
        }
        self.handle(index, taken)
    }

    /// Hides a shown pane: its attach ends and its reads are denied; nothing more is reported about it.
    pub(crate) fn hide(&mut self, machine_id: &str, session_id: &str, pane: Uuid) -> Vec<MobileEvent> {
        match self.find(machine_id, session_id, pane) {
            Some(index) => self.remove(index),
            None => Vec::new(),
        }
    }

    /// Hides every shown pane of `session_id`, whose stream was detached: its respawn counts go too.
    pub(crate) fn hide_session(&mut self, machine_id: &str, session_id: &str) -> Vec<MobileEvent> {
        self.counts.remove(&(machine_id.to_owned(), session_id.to_owned()));
        self.remove_where(|shown| shown.is(machine_id, session_id))
    }

    /// Hides every shown pane of `machine_id`.
    pub(crate) fn forget_machine(&mut self, machine_id: &str) -> Vec<MobileEvent> {
        self.counts.retain(|(machine, _), _| machine != machine_id);
        self.remove_where(|shown| shown.reference.machine_id == machine_id)
    }

    /// Hides every shown pane.
    pub(crate) fn clear(&mut self) {
        self.counts.clear();
        self.remove_where(|_| true);
    }

    fn remove_where(&mut self, matches: impl Fn(&Shown<C>) -> bool) -> Vec<MobileEvent> {
        let mut events = Vec::new();
        while let Some(index) = self.shown.iter().position(&matches) {
            events.extend(self.remove(index));
        }
        events
    }

    fn remove(&mut self, index: usize) -> Vec<MobileEvent> {
        let mut shown = self.shown.remove(index);
        let events = shown.leave();
        shown.slot.replace(None);
        let local = shown.local;
        self.book.prune(|kept, _| kept != local);
        events
    }

    /// `machine_id`'s connection is down: its panes' links end on their own, and wait for it.
    pub(crate) fn machine_down(&mut self, machine_id: &str) {
        for shown in &mut self.shown {
            if shown.reference.machine_id == machine_id {
                shown.connection = None;
                if let Some(retry) = shown.retry.take() {
                    retry.abort();
                }
            }
        }
    }

    /// `machine_id` is connected on `connection`: each of its panes whose link is gone, or runs on an earlier
    /// connection, attaches again into a fresh terminal, claiming nothing.
    pub(crate) fn machine_up(&mut self, machine_id: &str, connection: &Rc<C>) -> Vec<MobileEvent> {
        let mut again = Vec::new();
        for (index, shown) in self.shown.iter_mut().enumerate() {
            if shown.reference.machine_id != machine_id {
                continue;
            }
            shown.connection = Some(Rc::clone(connection));
            let on_another = shown
                .attachment
                .as_ref()
                .and_then(|attachment| attachment.connection.as_ref())
                .is_none_or(|on| !Rc::ptr_eq(on, connection));
            if matches!(shown.phase, Phase::Waiting)
                || (matches!(shown.phase, Phase::Opening | Phase::Live) && on_another)
            {
                shown.losses = 0;
                again.push(index);
            }
        }
        again
            .into_iter()
            .flat_map(|index| self.attach(index, LeadAttachment::new(false), true))
            .collect()
    }

    /// Takes the lead of a shown pane: it attaches again claiming it, covered until the attach reports. Nothing
    /// happens to a pane whose program ended, one with no connection, or one whose takeover is on its way.
    pub(crate) fn take_lead(&mut self, machine_id: &str, session_id: &str, pane: Uuid) -> Vec<MobileEvent> {
        let Some(index) = self.find(machine_id, session_id, pane) else {
            return Vec::new();
        };
        let shown = &self.shown[index];
        let ended = matches!(shown.phase, Phase::Checking | Phase::Exited | Phase::Gone);
        let claiming = shown
            .lead
            .as_ref()
            .is_some_and(|lead| lead.reattaching() && lead.attachment().claim);
        if ended || claiming || shown.connection.is_none() {
            return Vec::new();
        }
        self.attach(index, LeadAttachment::new(true), true)
    }

    /// What the session's origin reported about respawning its panes: an exited pane the origin respawned attaches
    /// again, attach-only (desktop's `apply_remote_respawns`).
    pub(crate) fn respawns(&mut self, reported: &Respawns) -> Vec<MobileEvent> {
        self.keep_counts(reported);
        let mut due = Vec::new();
        for (index, shown) in self.shown.iter().enumerate() {
            if !shown.is(&reported.machine_id, &reported.session_id) {
                continue;
            }
            let generation = reported.counts.generations.get(&shown.daemon).copied();
            if !reported.seed && generation.is_none() {
                continue;
            }
            let mark = Mark {
                epoch: reported.counts.epoch.clone(),
                generation: generation.unwrap_or_default(),
            };
            if self.book.observe(mark, shown.local, shown.session, reported.seed) {
                due.push(index);
            }
        }
        due.into_iter()
            .flat_map(|index| self.attach(index, LeadAttachment::new(false), true))
            .collect()
    }

    /// Keeps the session's latest counts: a snapshot's replace them, a respawn frame's update them (or replace them,
    /// from another origin run).
    fn keep_counts(&mut self, reported: &Respawns) {
        let key = (reported.machine_id.clone(), reported.session_id.clone());
        match self.counts.get_mut(&key) {
            Some(kept) if !reported.seed && kept.epoch == reported.counts.epoch => {
                kept.generations.extend(
                    reported
                        .counts
                        .generations
                        .iter()
                        .map(|(daemon, generation)| (daemon.clone(), *generation)),
                );
            }
            _ => {
                self.counts.insert(key, reported.counts.clone());
            }
        }
    }

    /// The user's answer to the clipboard read `read_id` of a shown pane; an answer to a read no longer open is
    /// dropped.
    pub(crate) fn answer_read(&mut self, reference: &PaneRef, pane: Uuid, read_id: u64, reply: ClipboardReply) {
        let Some(index) = self.find(&reference.machine_id, &reference.session_id, pane) else {
            return;
        };
        let shown = &mut self.shown[index];
        let Some(at) = shown.reads.iter().position(|read| read.id == read_id) else {
            return;
        };
        let read = shown.reads.remove(at);
        let answer = match reply {
            ClipboardReply::Allow { text } => ClipboardAnswer::Text(text),
            ClipboardReply::Deny => ClipboardAnswer::Denied,
        };
        if let Some(terminal) = shown.slot.terminal() {
            terminal.answer_clipboard_read(read.terminal, answer);
        }
    }

    /// Handles what an attach's threads or tasks noted.
    pub(crate) fn note(&mut self, note: Note) -> Handled {
        match note {
            Note::News(key) => self.news(key).into(),
            Note::Opened(key, opened) => self.opened(key, opened).into(),
            Note::Ended { key, end, gone } => self.ended(key, end, gone),
            Note::Retry(key) => self.retry(key).into(),
            Note::Sized(id) => self.sized(id).into(),
        }
    }

    /// The listing a [`GoneCheck`] asked for: `gone` when it no longer holds the session, which ends the pane for
    /// good; otherwise the pane's program just exited 1.
    pub(crate) fn gone_checked(&mut self, key: u64, gone: bool) -> Vec<MobileEvent> {
        let Some(index) = self
            .by_key(key)
            .filter(|index| self.shown[*index].phase == Phase::Checking)
        else {
            return Vec::new();
        };
        if !gone {
            return self.exited(index, Some(1), None);
        }
        let shown = &mut self.shown[index];
        shown.phase = Phase::Gone;
        let local = shown.local;
        let event = shown.event(|pane| MobileEvent::PaneEnded {
            pane,
            end: PaneEnd::Gone,
        });
        self.book.prune(|kept, _| kept != local);
        vec![event]
    }

    fn find(&self, machine_id: &str, session_id: &str, pane: Uuid) -> Option<usize> {
        self.shown
            .iter()
            .position(|shown| shown.is(machine_id, session_id) && shown.pane == pane)
    }

    fn by_key(&self, key: u64) -> Option<usize> {
        self.shown.iter().position(|shown| shown.key == key)
    }

    /// Attaches the pane at `index` again (or for the first time) into a fresh terminal, under `lead`; `cover` keeps
    /// it covered until the attach reports its role.
    fn attach(&mut self, index: usize, lead: LeadAttachment, cover: bool) -> Vec<MobileEvent> {
        self.last_key += 1;
        let key = self.last_key;
        let notes = self.notes.clone();
        let shown = &mut self.shown[index];
        let mut events = shown.leave();
        shown.key = key;
        shown.phase = Phase::Waiting;
        let built = shown.slot.size();
        if built.cols == 0 || built.rows == 0 {
            // the view has no grid yet: the pane attaches once it gives one ([`Panes::sized`])
            return events;
        }
        self.book.attach_started(shown.local, shown.session);
        let wake_notes = notes.clone();
        let wake: Wake = Arc::new(move || {
            // An actor that is gone has nothing left to draw.
            let _ = wake_notes.send(Note::News(key));
        });
        let (terminal, feed, out) = match ngterm_term::Pane::stream(&config(built, wake)) {
            Ok(made) => made,
            Err(error) => {
                // tried again on the backoff, in case memory is freed since
                let reason = error.to_string();
                events.push(shown.event(|pane| MobileEvent::PaneEnded {
                    pane,
                    end: PaneEnd::Lost { reason },
                }));
                if shown.connection.is_some() {
                    shown.retry_later(notes);
                }
                return events;
            }
        };
        shown.slot.install(Arc::new(terminal), built);
        let command = attach_tail(&shown.endpoint, &shown.daemon, Some(&lead));
        shown.lead = Some(LeadState::new(lead, cover));
        events.push(shown.event(|pane| MobileEvent::PaneAttached { pane }));
        events.push(shown.lead_event());
        let mut attachment = Attachment {
            connection: shown.connection.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            ends: Some((feed, out)),
            opening: None,
        };
        if let (Some(connection), Ok(command)) = (shown.connection.clone(), command) {
            let size = shown.slot.size();
            let opening = tokio::task::spawn_local(async move {
                let opened = connection.exec(&command, Some(size)).await;
                let _ = notes.send(Note::Opened(key, opened));
            });
            attachment.opening = Some(opening.abort_handle());
            shown.phase = Phase::Opening;
        }
        shown.attachment = Some(attachment);
        events
    }

    /// The attach's channel opened: the feeder and writer start. A channel of an attach since replaced is closed.
    fn opened(&mut self, key: u64, opened: Result<Channel, TransportError>) -> Vec<MobileEvent> {
        let Some(index) = self.by_key(key) else {
            if let Ok(channel) = opened {
                let _ = channel.writer.close();
            }
            return Vec::new();
        };
        let channel = match opened {
            Ok(channel) => channel,
            Err(error) => return self.lost(index, error.to_string()),
        };
        let shown = &mut self.shown[index];
        let terminal = shown.slot.terminal();
        let Some(attachment) = shown.attachment.as_mut() else {
            let _ = channel.writer.close();
            return Vec::new();
        };
        attachment.opening = None;
        let (Some((feed, out)), Some(terminal)) = (attachment.ends.take(), terminal) else {
            let _ = channel.writer.close();
            return Vec::new();
        };
        let stop = Arc::clone(&attachment.stop);
        let notes = self.notes.clone();
        let Channel { reader, writer } = channel;
        let closer = writer.clone();
        let started = thread::Builder::new()
            .name("ngterm-mobile-feed".to_owned())
            .spawn(move || feed_pane(reader, &feed, &notes, key))
            .and_then(|_| {
                thread::Builder::new()
                    .name("ngterm-mobile-write".to_owned())
                    .spawn(move || write_pane(&out, &writer, &stop, terminal))
            });
        if let Err(error) = started {
            let _ = closer.close();
            return self.lost(index, error.to_string());
        }
        shown.phase = Phase::Live;
        Vec::new()
    }

    fn ended(&mut self, key: u64, end: ChannelEnd, gone: bool) -> Handled {
        let Some(index) = self.by_key(key) else {
            return Handled::default();
        };
        match end {
            ChannelEnd::Lost(reason) => self.lost(index, reason).into(),
            ChannelEnd::Exit { code: Some(1), .. } if gone => {
                let shown = &mut self.shown[index];
                shown.phase = Phase::Checking;
                Handled {
                    events: shown.leave(),
                    check: Some(GoneCheck {
                        key,
                        machine_id: shown.reference.machine_id.clone(),
                        session_id: shown.reference.session_id.clone(),
                    }),
                }
            }
            ChannelEnd::Exit { code, signal } => self.exited(index, code, signal).into(),
        }
    }

    /// The attach's link went: the pane is covered and attaches again once it can, on the reconnect backoff when
    /// only its channel went.
    fn lost(&mut self, index: usize, reason: String) -> Vec<MobileEvent> {
        let notes = self.notes.clone();
        let shown = &mut self.shown[index];
        let on = shown
            .attachment
            .as_ref()
            .and_then(|attachment| attachment.connection.clone());
        let mut events = shown.leave();
        shown.phase = Phase::Waiting;
        events.push(shown.event(|pane| MobileEvent::PaneEnded {
            pane,
            end: PaneEnd::Lost { reason },
        }));
        let still_up = on
            .zip(shown.connection.as_ref())
            .is_some_and(|(on, current)| Rc::ptr_eq(&on, current));
        if still_up {
            shown.retry_later(notes);
        }
        events
    }

    fn retry(&mut self, key: u64) -> Vec<MobileEvent> {
        let Some(index) = self.by_key(key) else {
            return Vec::new();
        };
        let shown = &mut self.shown[index];
        shown.retry = None;
        if shown.phase != Phase::Waiting || shown.connection.is_none() {
            return Vec::new();
        }
        self.attach(index, LeadAttachment::new(false), true)
    }

    /// The view gave the pane `id` a grid while it held no terminal: a pane still waiting for its first attach, with
    /// its machine connected and no retry on its way, attaches now.
    fn sized(&mut self, id: u64) -> Vec<MobileEvent> {
        let Some(index) = self.shown.iter().position(|shown| shown.id == id) else {
            return Vec::new();
        };
        let shown = &self.shown[index];
        let waiting = shown.phase == Phase::Waiting && shown.attachment.is_none() && shown.retry.is_none();
        if !waiting || shown.connection.is_none() {
            return Vec::new();
        }
        let cover = shown.lead.is_some();
        self.attach(index, LeadAttachment::new(false), cover)
    }

    /// The program exited: reported, and attached again, attach-only, when the origin respawned it.
    fn exited(&mut self, index: usize, code: Option<u32>, signal: Option<String>) -> Vec<MobileEvent> {
        let shown = &mut self.shown[index];
        let mut events = shown.leave();
        shown.phase = Phase::Exited;
        shown.lead = None;
        events.push(shown.event(|pane| MobileEvent::PaneEnded {
            pane,
            end: PaneEnd::Exited { code, signal },
        }));
        if self.book.held(shown.local, shown.session) {
            events.extend(self.attach(index, LeadAttachment::new(false), true));
        }
        events
    }

    /// The terminal's news: its events, then that it has a frame to draw.
    fn news(&mut self, key: u64) -> Vec<MobileEvent> {
        let Some(index) = self.by_key(key) else {
            return Vec::new();
        };
        let Some(terminal) = self.shown[index].slot.terminal() else {
            return Vec::new();
        };
        self.handle(index, terminal.take_events())
    }

    /// The terminal events `taken` of the pane at `index`, then that it has a frame to draw.
    fn handle(&mut self, index: usize, taken: Vec<PaneEvent>) -> Vec<MobileEvent> {
        let mut events = Vec::new();
        for event in taken {
            if let Some(unowned) = self.pane_event(index, event, &mut events)
                && unowned
            {
                // the leader left: attach again without the claim, leading only if nobody claimed meanwhile
                events.extend(self.attach(index, LeadAttachment::new(false), true));
                return events;
            }
        }
        events.push(self.shown[index].event(|pane| MobileEvent::PaneDirty { pane }));
        events
    }

    /// One terminal event as the app sees it; `Some(unowned)` for a role report that changed the role.
    fn pane_event(&mut self, index: usize, event: PaneEvent, events: &mut Vec<MobileEvent>) -> Option<bool> {
        let shown = &mut self.shown[index];
        match event {
            PaneEvent::Title(title) if title.starts_with(LeadNotice::PREFIX) => {
                let notice = LeadNotice::parse(&title)?;
                let lead = shown.lead.as_mut()?;
                if !lead.apply(&notice) {
                    return None;
                }
                shown.losses = 0;
                self.book.acknowledged(shown.local);
                events.push(shown.lead_event());
                return Some(notice.role == LeadRole::Unowned);
            }
            PaneEvent::Title(title) => events.push(shown.event(|pane| MobileEvent::PaneTitle { pane, title })),
            PaneEvent::Bell => events.push(shown.event(|pane| MobileEvent::Bell { pane })),
            PaneEvent::ClipboardWrite(write) => events.push(clipboard_write(shown, write)),
            PaneEvent::ClipboardRead(request) => {
                self.last_read += 1;
                events.push(clipboard_read(shown, self.last_read, request));
            }
            _ => {}
        }
        None
    }
}

fn clipboard_write<C>(shown: &Shown<C>, write: ClipboardWrite) -> MobileEvent {
    let text = write
        .contents
        .into_iter()
        .find(|content| content.mime.starts_with("text/plain"))
        .map(|content| String::from_utf8_lossy(&content.data).into_owned());
    shown.event(|pane| MobileEvent::ClipboardWrite {
        pane,
        clipboard: write.location.into(),
        text,
    })
}

fn clipboard_read<C>(shown: &mut Shown<C>, id: u64, request: ClipboardReadRequest) -> MobileEvent {
    shown.reads.push(Read {
        id,
        terminal: request.id,
    });
    shown.event(|pane| MobileEvent::ClipboardReadRequest {
        pane,
        read_id: id,
        clipboard: request.location.into(),
        list_only: request.scope == ReadScope::ListOnly,
    })
}

/// The last bytes of a channel's output, enough to hold the gone guard's line and what the attach client writes after
/// it as it leaves.
#[derive(Debug, Default)]
pub(crate) struct Tail(Vec<u8>);

impl Tail {
    // the line, and room for the attach client's RESTORE (RIS and three OSC resets, ngterm-session's
    // `cli/attach.rs`) and a few more sequences
    const KEEP: usize = 2 * SESSION_GONE.len() + 128;

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
        let excess = self.0.len().saturating_sub(Self::KEEP);
        self.0.drain(..excess);
    }

    /// Whether the output ended with the gone guard's line (and its line ending), escape sequences aside: under a pty
    /// the attach client writes its terminal resets after the line, as it leaves.
    pub(crate) fn is_gone(&self) -> bool {
        let text = String::from_utf8_lossy(&without_escapes(&self.0)).into_owned();
        text.trim_end().ends_with(SESSION_GONE)
    }
}

/// `bytes` without their escape sequences: CSI (to its final byte), OSC, DCS, SOS, PM and APC (to BEL or ST), and
/// two-byte ones (`ESC c`). An unfinished sequence at the end is dropped too.
pub(crate) fn without_escapes(bytes: &[u8]) -> Vec<u8> {
    const ESC: u8 = 0x1b;
    const BEL: u8 = 0x07;
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != ESC {
            out.push(bytes[at]);
            at += 1;
            continue;
        }
        at = match bytes.get(at + 1) {
            None => bytes.len(),
            // CSI: parameters and intermediates, then a final byte in 0x40..=0x7e
            Some(b'[') => bytes[at + 2..]
                .iter()
                .position(|byte| (0x40..=0x7e).contains(byte))
                .map_or(bytes.len(), |end| at + 2 + end + 1),
            // strings ended by BEL or ST (ESC \)
            Some(b']' | b'P' | b'X' | b'^' | b'_') => {
                let body = at + 2;
                let mut end = bytes.len();
                let mut index = body;
                while index < bytes.len() {
                    if bytes[index] == BEL {
                        end = index + 1;
                        break;
                    }
                    if bytes[index] == ESC && bytes.get(index + 1) == Some(&b'\\') {
                        end = index + 2;
                        break;
                    }
                    index += 1;
                }
                end
            }
            Some(_) => at + 2,
        };
    }
    out
}

/// The feeder: the channel's output into the terminal until either ends; the channel's end, after the terminal's,
/// noted for the actor.
fn feed_pane(mut reader: ChannelReader, feed: &StreamFeed, notes: &Notes, key: u64) {
    let mut tail = Tail::default();
    loop {
        match reader.blocking_read() {
            ChannelOutput::Data(bytes) | ChannelOutput::Stderr(bytes) => {
                tail.push(&bytes);
                if !feed.deliver(&bytes) {
                    return;
                }
            }
            ChannelOutput::End(end) => {
                if let ChannelEnd::Exit { code, signal } = &end {
                    feed.end_of_output();
                    feed.exited(*code, signal.clone());
                }
                let gone = tail.is_gone();
                let _ = notes.send(Note::Ended { key, end, gone });
                return;
            }
        }
    }
}

/// The writer: what the terminal sends, down the channel, until the channel is gone or the attach stopped and the
/// queue went quiet; then the channel is closed. It holds the terminal so a reply queued as it stops still goes out.
fn write_pane(out: &StreamOut, writer: &ChannelWriter, stop: &AtomicBool, terminal: Arc<ngterm_term::Pane>) {
    let mut stopped_polls = 0;
    loop {
        // the machine isn't reading (ssh's window is shut): what the terminal queues stays in its accounting
        let sent = if writer.backlog() >= WRITE_BACKLOG {
            thread::sleep(WRITER_POLL / 4);
            true
        } else {
            match out.next(Some(WRITER_POLL)) {
                Ok(Some(Outgoing::Bytes(bytes))) => writer.write(&bytes),
                Ok(Some(Outgoing::Resize(size))) => writer.resize(size),
                Ok(None) if stop.load(Ordering::Acquire) => break,
                Ok(None) => true,
                Err(_) => break,
            }
        };
        if !sent {
            break;
        }
        if stop.load(Ordering::Acquire) {
            stopped_polls += 1;
            if stopped_polls >= STOPPED_POLLS {
                break;
            }
        }
    }
    let _ = writer.close();
    drop(terminal);
}
