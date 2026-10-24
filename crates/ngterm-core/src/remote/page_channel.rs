//! A handed-over page's channel on the presenter (upstream's `RemotePageChannel.swift`): this machine's end of the
//! page-serve channel to the machine that opened the page, which carries the page's file reads and relays its bridge
//! requests ([`super::page_serve`] is the other end).
//!
//! Toolkit-neutral and single-threaded: the owner launches the helper that carries the channel (an ssh running
//! `ngtermctl session overlay page-serve TOKEN` on the origin, `ngterm_app::page_channel`), hands every line it reads to
//! [`ReplicaChannel::receive`] and acts on the [`ChannelEvent`]s that come back. The helper's reader takes one of the
//! channel's [`Permits`] before it hands a line over and stops at the bound, so a busy main loop backs the origin up
//! into its own pipe; a line gives its permit back once handled, and a file chunk only once [`ReplicaChannel::pull`]
//! took it.
//!
//! At most [`RESOURCE_LIMIT`] resource requests are in flight, the rest wait here; a request's slot is freed only by
//! its terminal frame, which a cancel also produces. Bridge calls have [`BRIDGE_LIMIT`] slots of their own.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use super::page_serve::{self, BRIDGE_LIMIT, PageServeFrame, RESOURCE_LIMIT};

/// Lines the helper's reader may hold before the channel handled them (upstream's `inboundLimit`).
pub const INBOUND_LIMIT: usize = 64;
/// Lines the helper's writer holds before a send is refused and the channel closes (upstream's `limits`).
pub const MAX_PENDING_LINES: usize = 32;

const CLOSED: &str = "page channel closed";
const TOO_LARGE: &str = "request too large for the page channel";
const CANCELLED: &str = "cancelled";
/// What a resource whose answer ended before its head fails with (upstream's `finish` on a success).
const EMPTY_ANSWER: &str = "empty answer";

/// Bounds the inbound lines a channel holds (upstream's `RemotePagePermits`): the reader takes one per line and stops
/// at the bound, a line gives its permit back once handled, and a file chunk only once it is consumed.
#[derive(Debug)]
pub struct Permits {
    state: Mutex<PermitState>,
    wake: Condvar,
}

#[derive(Debug)]
struct PermitState {
    free: usize,
    closed: bool,
}

impl Permits {
    #[must_use]
    pub fn new(count: usize) -> Self {
        Self {
            state: Mutex::new(PermitState {
                free: count,
                closed: false,
            }),
            wake: Condvar::new(),
        }
    }

    /// Waits for a permit; false once closed, when nothing more is delivered.
    pub fn take(&self) -> bool {
        let mut state = self.lock();
        while !state.closed && state.free == 0 {
            state = self.wake.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
        if state.closed {
            return false;
        }
        state.free -= 1;
        true
    }

    /// Takes a permit when one is free, without waiting: the tests' in-process readers.
    #[cfg(test)]
    pub(crate) fn try_take(&self) -> bool {
        let mut state = self.lock();
        if state.closed || state.free == 0 {
            return false;
        }
        state.free -= 1;
        true
    }

    pub fn give(&self) {
        let mut state = self.lock();
        state.free += 1;
        self.wake.notify_all();
    }

    pub fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        self.wake.notify_all();
    }

    /// The permits free now.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn free(&self) -> usize {
        self.lock().free
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PermitState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The helper carrying one channel, as the channel writes to it.
pub trait ChannelLink {
    /// Queues one line, its newline included; false when the helper stopped taking lines, which closes the channel.
    fn send(&self, line: Vec<u8>) -> bool;
    /// Stops the helper; nothing it reports afterwards is read.
    fn stop(&self);
}

/// Why a call over the channel failed (upstream's `RemotePageChannel.Failure`, with the cancellation its tasks throw).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelFailure {
    Closed,
    TooLarge,
    /// The origin's own failure.
    Refused(String),
    /// The request was withdrawn here.
    Cancelled,
}

impl fmt::Display for ChannelFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str(CLOSED),
            Self::TooLarge => f.write_str(TOO_LARGE),
            Self::Refused(reason) => f.write_str(reason),
            Self::Cancelled => f.write_str(CANCELLED),
        }
    }
}

/// A request's id on the channel.
pub type RequestId = i64;

/// What the channel tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelEvent {
    /// A resource's answer starts.
    Head { id: RequestId, mime_type: String },
    /// A chunk of a resource's body waits: [`ReplicaChannel::pull`] it.
    Data { id: RequestId },
    /// A resource request ended with nothing left to pull: whole, or failed.
    Finished {
        id: RequestId,
        result: Result<(), ChannelFailure>,
    },
    /// A bridge call's answer: the origin's reply as the JSON text it sent.
    Reply {
        id: RequestId,
        result: Result<String, ChannelFailure>,
    },
    /// The channel ended, once, whichever side ended it.
    Closed { reason: String },
}

/// What [`ReplicaChannel::pull`] gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pull {
    Chunk(Vec<u8>),
    /// The body is whole, or failed: the request is gone.
    Done(Result<(), ChannelFailure>),
    /// Nothing yet.
    Wait,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Resource,
    Bridge,
}

/// A request waiting for a slot.
#[derive(Debug)]
enum Queued {
    Resource { path: String, revision: u64 },
    Bridge { request: String },
}

/// An admitted resource request.
#[derive(Debug)]
struct Transfer {
    /// The page load that asked; an answer for a load the page left is dropped.
    revision: u64,
    head: bool,
    /// Chunks not pulled yet, each holding its inbound permit.
    buffer: VecDeque<Vec<u8>>,
    terminal: Option<Result<(), ChannelFailure>>,
    cancelled: bool,
}

#[derive(Debug, Default)]
struct Slots {
    used: usize,
    waiting: VecDeque<(RequestId, Queued)>,
}

/// This machine's end of one page's channel.
pub struct ReplicaChannel {
    link: Box<dyn ChannelLink>,
    permits: Arc<Permits>,
    next_id: RequestId,
    transfers: HashMap<RequestId, Transfer>,
    bridges: HashSet<RequestId>,
    resources: Slots,
    bridge_slots: Slots,
    closed: bool,
}

impl fmt::Debug for ReplicaChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplicaChannel")
            .field("transfers", &self.transfers.len())
            .field("bridges", &self.bridges.len())
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl ReplicaChannel {
    /// A channel over `link`, its reader bounded by `permits`.
    #[must_use]
    pub fn new(link: Box<dyn ChannelLink>, permits: Arc<Permits>) -> Self {
        Self {
            link,
            permits,
            next_id: 1,
            transfers: HashMap::new(),
            bridges: HashSet::new(),
            resources: Slots::default(),
            bridge_slots: Slots::default(),
            closed: false,
        }
    }

    /// A fresh set of permits for a channel's reader.
    #[must_use]
    pub fn permits() -> Arc<Permits> {
        Arc::new(Permits::new(INBOUND_LIMIT))
    }

    /// Asks the origin for its file at `path`, for the page load `revision` names. Sent at once while a slot is free,
    /// queued here otherwise. Its answer comes as [`ChannelEvent::Head`], then [`ChannelEvent::Data`] to pull, then
    /// the end ([`Pull::Done`] or [`ChannelEvent::Finished`]).
    ///
    /// # Errors
    ///
    /// [`ChannelFailure::Closed`] once the channel ended.
    pub fn fetch(&mut self, path: &str, revision: u64) -> Result<(RequestId, Vec<ChannelEvent>), ChannelFailure> {
        if self.closed {
            return Err(ChannelFailure::Closed);
        }
        let id = self.allocate();
        let events = self.admit(
            id,
            Queued::Resource {
                path: path.to_owned(),
                revision,
            },
        );
        Ok((id, events))
    }

    /// Relays a page's request as the raw JSON it sent; the origin's reply comes as [`ChannelEvent::Reply`].
    ///
    /// # Errors
    ///
    /// [`ChannelFailure::Closed`] once the channel ended; [`ChannelFailure::TooLarge`] for a request no frame holds,
    /// which fails alone and leaves the channel up.
    pub fn bridge(&mut self, request: &str) -> Result<(RequestId, Vec<ChannelEvent>), ChannelFailure> {
        if self.closed {
            return Err(ChannelFailure::Closed);
        }
        let id = self.allocate();
        let frame = PageServeFrame::Bridge {
            id,
            request: request.to_owned(),
        };
        if page_serve::encode(&frame).is_err() {
            return Err(ChannelFailure::TooLarge);
        }
        let events = self.admit(
            id,
            Queued::Bridge {
                request: request.to_owned(),
            },
        );
        Ok((id, events))
    }

    /// Withdraws resource request `id`, whose answer nobody wants any more. One still waiting here leaves without a
    /// trace on the origin; an admitted one is told to the origin and keeps its slot until the origin's terminal frame,
    /// while what it buffered is dropped at once, freeing those permits so the reader can reach that frame. What comes
    /// back is the channel's close, should the helper refuse the cancel.
    pub fn cancel(&mut self, id: RequestId) -> Vec<ChannelEvent> {
        if let Some(index) = self.resources.waiting.iter().position(|(queued, _)| *queued == id) {
            self.resources.waiting.remove(index);
            return Vec::new();
        }
        let Some(transfer) = self.transfers.get_mut(&id) else {
            return Vec::new();
        };
        if transfer.cancelled {
            return Vec::new();
        }
        transfer.cancelled = true;
        let dropped = std::mem::take(&mut transfer.buffer).len();
        let ended = transfer.terminal.is_some();
        for _ in 0..dropped {
            self.permits.give();
        }
        if ended {
            self.transfers.remove(&id);
            return Vec::new();
        }
        self.send(&PageServeFrame::Cancel { id }).err().unwrap_or_default()
    }

    /// The page loads `revision` now: every request a load before it made is withdrawn, its owner told it was
    /// cancelled.
    pub fn revision_moved(&mut self, revision: u64) -> Vec<ChannelEvent> {
        let mut events = Vec::new();
        let queued: Vec<RequestId> = self
            .resources
            .waiting
            .iter()
            .filter(|(_, queued)| matches!(queued, Queued::Resource { revision: asked, .. } if *asked < revision))
            .map(|(id, _)| *id)
            .collect();
        let mut admitted: Vec<RequestId> = self
            .transfers
            .iter()
            .filter(|(_, transfer)| !transfer.cancelled && transfer.revision < revision)
            .map(|(id, _)| *id)
            .collect();
        admitted.sort_unstable();
        for id in queued.into_iter().chain(admitted) {
            if self.closed {
                break;
            }
            let closing = self.cancel(id);
            events.push(ChannelEvent::Finished {
                id,
                result: Err(ChannelFailure::Cancelled),
            });
            events.extend(closing);
        }
        events
    }

    /// The next piece of resource `id`'s body: a chunk gives its permit back as it is taken.
    pub fn pull(&mut self, id: RequestId) -> Pull {
        let Some(transfer) = self.transfers.get_mut(&id) else {
            return Pull::Done(Err(ChannelFailure::Closed));
        };
        if transfer.cancelled {
            return Pull::Done(Err(ChannelFailure::Cancelled));
        }
        if let Some(chunk) = transfer.buffer.pop_front() {
            self.permits.give();
            return Pull::Chunk(chunk);
        }
        match transfer.terminal.clone() {
            Some(result) => {
                self.transfers.remove(&id);
                Pull::Done(result)
            }
            None => Pull::Wait,
        }
    }

    /// One line the helper read, without its newline, holding the permit its reader took.
    pub fn receive(&mut self, line: &[u8]) -> Vec<ChannelEvent> {
        if self.closed {
            self.permits.give();
            return Vec::new();
        }
        let Ok(frame) = page_serve::decode(line) else {
            self.permits.give();
            return self.close("a malformed frame");
        };
        let mut events = Vec::new();
        match frame {
            PageServeFrame::Head { id, mime_type } => {
                self.permits.give();
                if let Some(transfer) = self.live(id) {
                    transfer.head = true;
                    events.push(ChannelEvent::Head { id, mime_type });
                }
            }
            PageServeFrame::Chunk { id, data } => match self.live(id) {
                Some(transfer) => {
                    transfer.buffer.push_back(data);
                    events.push(ChannelEvent::Data { id });
                }
                None => self.permits.give(),
            },
            PageServeFrame::End { id } => {
                self.permits.give();
                self.end(id, Ok(()), &mut events);
            }
            PageServeFrame::Failure { id, error } => {
                self.permits.give();
                if self.bridges.remove(&id) {
                    self.release(Kind::Bridge, &mut events);
                    events.push(ChannelEvent::Reply {
                        id,
                        result: Err(ChannelFailure::Refused(error)),
                    });
                } else {
                    self.end(id, Err(ChannelFailure::Refused(error)), &mut events);
                }
            }
            PageServeFrame::Reply { id, response } => {
                self.permits.give();
                if self.bridges.remove(&id) {
                    self.release(Kind::Bridge, &mut events);
                    events.push(ChannelEvent::Reply {
                        id,
                        result: Ok(response),
                    });
                }
            }
            PageServeFrame::Resource { .. } | PageServeFrame::Cancel { .. } | PageServeFrame::Bridge { .. } => {
                self.permits.give();
                events.extend(self.close("a frame only this machine sends"));
            }
        }
        events
    }

    /// Ends the channel once: the helper stops, every waiting call fails and every held permit is freed.
    pub fn close(&mut self, reason: &str) -> Vec<ChannelEvent> {
        if self.closed {
            return Vec::new();
        }
        self.closed = true;
        tracing::info!(reason, "page channel closed");
        self.permits.close();
        self.link.stop();
        let mut events = Vec::new();
        let mut transfers: Vec<(RequestId, Transfer)> = self.transfers.drain().collect();
        transfers.sort_by_key(|(id, _)| *id);
        for (id, transfer) in transfers {
            for _ in 0..transfer.buffer.len() {
                self.permits.give();
            }
            if !transfer.cancelled {
                events.push(ChannelEvent::Finished {
                    id,
                    result: Err(ChannelFailure::Closed),
                });
            }
        }
        let mut bridges: Vec<RequestId> = self.bridges.drain().collect();
        bridges.sort_unstable();
        for id in bridges {
            events.push(ChannelEvent::Reply {
                id,
                result: Err(ChannelFailure::Closed),
            });
        }
        for (id, queued) in self
            .resources
            .waiting
            .drain(..)
            .chain(self.bridge_slots.waiting.drain(..))
        {
            events.push(match queued {
                Queued::Resource { .. } => ChannelEvent::Finished {
                    id,
                    result: Err(ChannelFailure::Closed),
                },
                Queued::Bridge { .. } => ChannelEvent::Reply {
                    id,
                    result: Err(ChannelFailure::Closed),
                },
            });
        }
        events.push(ChannelEvent::Closed {
            reason: reason.to_owned(),
        });
        events
    }

    fn allocate(&mut self) -> RequestId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn slots(&mut self, kind: Kind) -> (&mut Slots, usize) {
        match kind {
            Kind::Resource => (&mut self.resources, RESOURCE_LIMIT),
            Kind::Bridge => (&mut self.bridge_slots, BRIDGE_LIMIT),
        }
    }

    /// Takes a slot for `queued` and sends it, or queues it for the next slot freed.
    fn admit(&mut self, id: RequestId, queued: Queued) -> Vec<ChannelEvent> {
        let kind = match queued {
            Queued::Resource { .. } => Kind::Resource,
            Queued::Bridge { .. } => Kind::Bridge,
        };
        let (slots, limit) = self.slots(kind);
        if slots.used >= limit {
            slots.waiting.push_back((id, queued));
            return Vec::new();
        }
        slots.used += 1;
        self.start(id, queued)
    }

    /// Sends an admitted request; one no frame holds fails alone and frees its slot, and a helper that stops taking
    /// lines closes the channel.
    fn start(&mut self, id: RequestId, queued: Queued) -> Vec<ChannelEvent> {
        let (kind, frame, revision) = match queued {
            Queued::Resource { path, revision } => (Kind::Resource, PageServeFrame::Resource { id, path }, revision),
            Queued::Bridge { request } => (Kind::Bridge, PageServeFrame::Bridge { id, request }, 0),
        };
        let Ok(line) = page_serve::encode(&frame) else {
            let mut events = vec![match kind {
                Kind::Resource => ChannelEvent::Finished {
                    id,
                    result: Err(ChannelFailure::TooLarge),
                },
                Kind::Bridge => ChannelEvent::Reply {
                    id,
                    result: Err(ChannelFailure::TooLarge),
                },
            }];
            self.release(kind, &mut events);
            return events;
        };
        match kind {
            Kind::Resource => {
                self.transfers.insert(
                    id,
                    Transfer {
                        revision,
                        head: false,
                        buffer: VecDeque::new(),
                        terminal: None,
                        cancelled: false,
                    },
                );
            }
            Kind::Bridge => {
                self.bridges.insert(id);
            }
        }
        match self.write(line) {
            Ok(()) => Vec::new(),
            Err(events) => events,
        }
    }

    /// A freed slot passes straight to the first request waiting for one.
    fn release(&mut self, kind: Kind, events: &mut Vec<ChannelEvent>) {
        let (slots, _) = self.slots(kind);
        match slots.waiting.pop_front() {
            Some((id, queued)) => events.extend(self.start(id, queued)),
            None => slots.used = slots.used.saturating_sub(1),
        }
    }

    /// A transfer whose answer is still wanted.
    fn live(&mut self, id: RequestId) -> Option<&mut Transfer> {
        self.transfers.get_mut(&id).filter(|transfer| !transfer.cancelled)
    }

    /// Resource `id`'s terminal frame: its slot frees, and its owner hears the end unless it withdrew the request.
    fn end(&mut self, id: RequestId, result: Result<(), ChannelFailure>, events: &mut Vec<ChannelEvent>) {
        let Some(transfer) = self
            .transfers
            .get_mut(&id)
            .filter(|transfer| transfer.terminal.is_none())
        else {
            return;
        };
        let cancelled = transfer.cancelled;
        let result = match result {
            Ok(()) if !transfer.head => Err(ChannelFailure::Refused(EMPTY_ANSWER.to_owned())),
            other => other,
        };
        let whole = transfer.buffer.is_empty() || result.is_err();
        if cancelled || whole {
            if let Some(transfer) = self.transfers.remove(&id) {
                for _ in 0..transfer.buffer.len() {
                    self.permits.give();
                }
            }
            if !cancelled {
                events.push(ChannelEvent::Finished { id, result });
            }
        } else {
            transfer.terminal = Some(result);
        }
        self.release(Kind::Resource, events);
    }

    /// Writes one frame (a cancel is always small enough); a helper that stopped taking lines closes the channel.
    fn send(&mut self, frame: &PageServeFrame) -> Result<(), Vec<ChannelEvent>> {
        match page_serve::encode(frame) {
            Ok(line) => self.write(line),
            Err(_) => Ok(()),
        }
    }

    fn write(&mut self, line: Vec<u8>) -> Result<(), Vec<ChannelEvent>> {
        if self.closed {
            return Err(Vec::new());
        }
        if self.link.send(line) {
            Ok(())
        } else {
            Err(self.close("the channel stopped taking requests"))
        }
    }
}

#[cfg(test)]
#[path = "page_channel_tests.rs"]
mod tests;
