//! Shown panes: each one an `ngterm-session attach` of the pane's daemon on an exec channel with a pty, feeding an
//! `ngterm-term` stream pane (`Pane::stream`) the app draws and types into ([`crate::api::Pane`]).
//!
//! - **Threads.** Once the channel opens, a feeder thread reads it into the pane's `StreamFeed` (which blocks while
//!   the terminal is behind, holding the channel back) and a writer thread sends what the pane's `StreamOut` gives
//!   (input, the terminal's replies, resizes) down the channel. Both report back to the actor as `Note`s, as the
//!   pane's wake does; a note of an attach since replaced is dropped. A stopped writer sends what is still queued (a
//!   denied clipboard read's reply) and closes the channel once the queue has been quiet for [`WRITER_POLL`].
//! - **Leadership.** The attach tail carries a `LeadAttachment`: the first attach and every automatic one claim
//!   nothing (they lead only a session nobody leads), [`crate::MobileCore::take_lead`] claims. The attach client's
//!   role reports arrive as titles under `LeadNotice::PREFIX`: filtered through the pane's `LeadState` into
//!   [`MobileEvent::PaneLead`], never shown as a title. A follower is covered; an unowned pane attaches again.
//! - **How an attach ends** ([`PaneEnd`]): a *lost* link covers the pane, and it attaches again into a fresh terminal
//!   once its machine is connected (at once on reconnect; on the reconnect backoff when only the channel went);
//!   an *exit* reports the program's status after `end_of_output` and `exited`, and attaches again, attach-only, when
//!   the origin reports the pane's shell respawned (`RemoteRespawnBook`, fed by the session's presentation stream);
//!   an exit 1 after the gone guard's [`SESSION_GONE`] line is checked against a fresh listing, and a session no
//!   longer listed is *gone*: final.
//! - **Clipboard reads** (OSC 52) stop the pane's output until answered: each raises
//!   [`MobileEvent::ClipboardReadRequest`] under an id of the core's own, answered through
//!   [`crate::MobileCore::answer_clipboard_read`]. Hiding the pane, losing its link or its program's exit deny any read
//!   still open and dismiss it ([`MobileEvent::ClipboardReadDismiss`]).
//! - **Kitty images** are off (`kitty_image_limit` 0): the attach client's probe reports no support.

mod records;
#[cfg(test)]
mod tests;

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use ngterm_core::live::lead::{LeadAttachment, LeadNotice, LeadRole, LeadState};
use ngterm_core::pane_env::uuid_string;
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
    notes: Notes,
    last_key: u64,
    last_read: u64,
}

impl<C: Connection> Panes<C> {
    pub(crate) fn new(notes: Notes) -> Self {
        Self {
            shown: Vec::new(),
            book: RemoteRespawnBook::default(),
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
        let slot = Arc::new(PaneSlot::new(size));
        self.shown.push(Shown {
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

    /// Hides a shown pane: its attach ends and its reads are denied; nothing more is reported about it.
    pub(crate) fn hide(&mut self, machine_id: &str, session_id: &str, pane: Uuid) -> Vec<MobileEvent> {
        match self.find(machine_id, session_id, pane) {
            Some(index) => self.remove(index),
            None => Vec::new(),
        }
    }

    /// Hides every shown pane of `session_id`.
    pub(crate) fn hide_session(&mut self, machine_id: &str, session_id: &str) -> Vec<MobileEvent> {
        self.remove_where(|shown| shown.is(machine_id, session_id))
    }

    /// Hides every shown pane of `machine_id`.
    pub(crate) fn forget_machine(&mut self, machine_id: &str) -> Vec<MobileEvent> {
        self.remove_where(|shown| shown.reference.machine_id == machine_id)
    }

    /// Hides every shown pane.
    pub(crate) fn clear(&mut self) {
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
        let pane = shown.pane;
        self.book.prune(|kept, _| kept != pane);
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
            if self.book.observe(mark, shown.pane, shown.session, reported.seed) {
                due.push(index);
            }
        }
        due.into_iter()
            .flat_map(|index| self.attach(index, LeadAttachment::new(false), true))
            .collect()
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
        let pane = shown.pane;
        let event = shown.event(|pane| MobileEvent::PaneEnded {
            pane,
            end: PaneEnd::Gone,
        });
        self.book.prune(|kept, _| kept != pane);
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
        self.book.attach_started(shown.pane, shown.session);
        let wake_notes = notes.clone();
        let wake: Wake = Arc::new(move || {
            // An actor that is gone has nothing left to draw.
            let _ = wake_notes.send(Note::News(key));
        });
        let built = shown.slot.size();
        let (terminal, feed, out) = match ngterm_term::Pane::stream(&config(built, wake)) {
            Ok(made) => made,
            Err(error) => {
                // tried again on the backoff, so a grid the view fixes since (or memory freed) gets its attach
                shown.phase = Phase::Waiting;
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
        shown.phase = Phase::Waiting;
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
        if self.book.held(shown.pane, shown.session) {
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
        let mut events = Vec::new();
        for event in terminal.take_events() {
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
                self.book.acknowledged(shown.pane);
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

/// The last bytes of a channel's output, enough to hold the gone guard's line.
#[derive(Debug, Default)]
pub(crate) struct Tail(Vec<u8>);

impl Tail {
    const KEEP: usize = 2 * SESSION_GONE.len() + 8;

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
        let excess = self.0.len().saturating_sub(Self::KEEP);
        self.0.drain(..excess);
    }

    /// Whether the output ended with the gone guard's line (and its line ending).
    pub(crate) fn is_gone(&self) -> bool {
        let text = String::from_utf8_lossy(&self.0);
        text.trim_end().ends_with(SESSION_GONE)
    }
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
        let sent = match out.next(Some(WRITER_POLL)) {
            Ok(Some(Outgoing::Bytes(bytes))) => writer.write(&bytes),
            Ok(Some(Outgoing::Resize(size))) => writer.resize(size),
            Ok(None) if stop.load(Ordering::Acquire) => break,
            Ok(None) => true,
            Err(_) => break,
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
