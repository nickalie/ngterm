//! The page-serve channel on the origin: the connection a presenter's helper opens back to this machine for one page
//! it was handed (upstream's `PageServeFrames.swift`, `RemotePageResources.swift` and `ControlServer+PageServe.swift`).
//!
//! After `session.overlay.page.serve` admits the helper's claim, its connection carries newline-delimited JSON frames
//! that multiplex the page's resource transfers and its bridge calls by request id. A resource is read here off the
//! main loop, from the page's own file or from inside its grant, in chunks of at most [`MAX_CHUNK_BYTES`]; a bridge
//! request runs here against the page's place at that moment, as a local page's request would. The helper's
//! connection closing, or sending what a helper never sends, ends the page and keeps the presenter.
//!
//! The frontend owns the connection: it hands the core each adopted one as a [`PageChannel`], calls
//! [`PageServeStream::take`] with every line on the connection's own thread (a cancel applies at once and a request
//! past its limit is refused at once, so nothing there waits), posts the [`PageServeWork`] it returns to the main loop,
//! and runs the request a bridge call hands back ([`PageServeCall`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ngterm_proto::{Request, Response};
use serde_json::Value;
use uuid::Uuid;

use super::frames::{self, FrameError};
use crate::Core;
use crate::html::HtmlSource;
use crate::html::bridge::{self, HtmlBridgePage};
use crate::pane_env::uuid_string;

/// The largest chunk body a transfer sends, and the largest one either side accepts (upstream's `maxChunkBytes`).
pub const MAX_CHUNK_BYTES: usize = 64 * 1024;
/// The longest line either side accepts: a presentation frame's (upstream's `PageServeCodec.maxFrameBytes`).
pub const MAX_FRAME_BYTES: usize = frames::MAX_FRAME_BYTES;
/// Lines the channel's writer holds before a non-blocking send is refused (upstream's `pageServeLimits`).
pub const MAX_PENDING_LINES: usize = 8;
/// How long one blocked write may take before the helper counts as stalled.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Resource transfers a channel takes in at once (upstream's `pageServeResourceLimit`).
pub const RESOURCE_LIMIT: usize = 8;
/// Bridge calls a channel takes in at once (upstream's `pageServeBridgeLimit`).
pub const BRIDGE_LIMIT: usize = 8;

/// The failure a request past its kind's limit gets (upstream's `PageServeFrame.busy`).
pub const BUSY: &str = "too many requests in flight";
/// The failure that ends a transfer the presenter cancelled (upstream's `PageServeFrame.cancelled`).
pub const CANCELLED: &str = "cancelled";
/// The failure of a bridge call the frontend could not run.
pub const CONTROL_UNAVAILABLE: &str = "control is unavailable";
const OUTSIDE_FOLDER: &str = "file is outside the page's folder";
const UNREADABLE: &str = "file cannot be read";
const PAGE_CLOSED: &str = "page closed";
const ANSWER_TOO_LARGE: &str = "answer too large for the channel";
const UNENCODABLE: &str = "reply could not be encoded";

// MARK: - frames

/// One line of a page channel (upstream's `PageServeFrame`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageServeFrame {
    /// Asks for one of the page's files by its path on the origin.
    Resource {
        id: i64,
        path: String,
    },
    /// Starts a resource's answer; chunks follow, then [`PageServeFrame::End`].
    Head {
        id: i64,
        mime_type: String,
    },
    Chunk {
        id: i64,
        data: Vec<u8>,
    },
    End {
        id: i64,
    },
    /// Ends a resource or bridge request the other side could not serve.
    Failure {
        id: i64,
        error: String,
    },
    /// Withdraws a resource request whose answer is no longer wanted.
    Cancel {
        id: i64,
    },
    /// A page's control request as the raw JSON the page sent: only the origin decodes it, so a presenter on an older
    /// protocol neither refuses nor strips a command or argument it does not know.
    Bridge {
        id: i64,
        request: String,
    },
    /// The origin's raw JSON response, passed to the page as received.
    Reply {
        id: i64,
        response: String,
    },
}

impl PageServeFrame {
    /// The request the frame belongs to.
    #[must_use]
    pub fn id(&self) -> i64 {
        match self {
            Self::Resource { id, .. }
            | Self::Head { id, .. }
            | Self::Chunk { id, .. }
            | Self::End { id }
            | Self::Failure { id, .. }
            | Self::Cancel { id }
            | Self::Bridge { id, .. }
            | Self::Reply { id, .. } => *id,
        }
    }

    /// The frame as a JSON object with its keys sorted, as upstream's encoder writes it.
    fn object(&self) -> BTreeMap<&'static str, Value> {
        let mut object = BTreeMap::new();
        object.insert("id", Value::from(self.id()));
        let (kind, field) = match self {
            Self::Resource { path, .. } => ("resource", Some(("path", Value::from(path.as_str())))),
            Self::Head { mime_type, .. } => ("head", Some(("mimeType", Value::from(mime_type.as_str())))),
            Self::Chunk { data, .. } => ("chunk", Some(("data", Value::from(STANDARD.encode(data))))),
            Self::End { .. } => ("end", None),
            Self::Failure { error, .. } => ("failure", Some(("error", Value::from(error.as_str())))),
            Self::Cancel { .. } => ("cancel", None),
            Self::Bridge { request, .. } => ("bridge", Some(("request", Value::from(request.as_str())))),
            Self::Reply { response, .. } => ("reply", Some(("response", Value::from(response.as_str())))),
        };
        object.insert("kind", Value::from(kind));
        if let Some((key, value)) = field {
            object.insert(key, value);
        }
        object
    }

    /// The frame a decoded object spells.
    fn from_object(object: &serde_json::Map<String, Value>) -> Result<Self, String> {
        let id = object
            .get("id")
            .and_then(Value::as_i64)
            .ok_or_else(|| "missing or invalid id".to_owned())?;
        let kind = object
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing or invalid kind".to_owned())?;
        let text = |key: &str| {
            object
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("missing or invalid {key}"))
        };
        Ok(match kind {
            "resource" => Self::Resource {
                id,
                path: text("path")?,
            },
            "head" => Self::Head {
                id,
                mime_type: text("mimeType")?,
            },
            "chunk" => Self::Chunk {
                id,
                data: STANDARD
                    .decode(text("data")?)
                    .map_err(|error| format!("invalid data: {error}"))?,
            },
            "end" => Self::End { id },
            "failure" => Self::Failure {
                id,
                error: text("error")?,
            },
            "cancel" => Self::Cancel { id },
            "bridge" => Self::Bridge {
                id,
                request: text("request")?,
            },
            "reply" => Self::Reply {
                id,
                response: text("response")?,
            },
            other => return Err(format!("unknown kind {other}")),
        })
    }
}

/// Renders `frame` as one line, newline included (upstream's `PageServeCodec.encode`).
///
/// # Errors
///
/// A chunk body over [`MAX_CHUNK_BYTES`], or a line that would reach [`MAX_FRAME_BYTES`].
pub fn encode(frame: &PageServeFrame) -> Result<Vec<u8>, FrameError> {
    check_chunk(frame)?;
    let mut line = serde_json::to_vec(&frame.object()).map_err(|error| FrameError::Malformed(error.to_string()))?;
    if line.len() >= MAX_FRAME_BYTES {
        return Err(FrameError::Oversize(line.len()));
    }
    line.push(b'\n');
    Ok(line)
}

/// Parses one line, without its newline (upstream's `PageServeCodec.decode`).
///
/// # Errors
///
/// A line over [`MAX_FRAME_BYTES`] (refused before it is parsed), one that is not a frame, or a chunk body over
/// [`MAX_CHUNK_BYTES`].
pub fn decode(line: &[u8]) -> Result<PageServeFrame, FrameError> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Oversize(line.len()));
    }
    let value: Value = serde_json::from_slice(line).map_err(|error| FrameError::Malformed(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| FrameError::Malformed("not an object".to_owned()))?;
    let frame = PageServeFrame::from_object(object).map_err(FrameError::Malformed)?;
    check_chunk(&frame)?;
    Ok(frame)
}

fn check_chunk(frame: &PageServeFrame) -> Result<(), FrameError> {
    match frame {
        PageServeFrame::Chunk { data, .. } if data.len() > MAX_CHUNK_BYTES => Err(FrameError::Oversize(data.len())),
        _ => Ok(()),
    }
}

// MARK: - resources

/// The file on this machine a channel's resource request may read, `None` when it may not (upstream's
/// `RemotePageResources.resolve`): with no grant only the page's own file, with a grant any file inside it but not the
/// grant itself. A replica asks by the origin path its scheme URL carries, so a request is an absolute path. Both sides
/// are standardized and their symlinks resolved before the check, so a `..` or a link that escapes the folder is
/// refused. A URL page serves nothing.
#[must_use]
pub fn resolve(requested: &str, source: &HtmlSource) -> Option<PathBuf> {
    let HtmlSource::File { path, grant_root } = source else {
        return None;
    };
    let requested = Path::new(requested);
    if !requested.is_absolute() {
        return None;
    }
    let file = real(requested);
    match grant_root {
        None => (file == real(Path::new(path))).then_some(file),
        Some(root) => {
            let root = real(Path::new(root));
            (file.starts_with(&root) && file != root).then_some(file)
        }
    }
}

/// `path` standardized (`.` dropped, `..` taking the previous name off, as Foundation's `standardizedFileURL`), then
/// with the symlinks of its longest existing ancestor resolved, as `resolvingSymlinksInPath` leaves a path that does
/// not exist yet. Both sides of [`resolve`]'s comparison go through it, so a Windows path's `\\?\` prefix, which
/// `canonicalize` adds, is on both or neither.
fn real(path: &Path) -> PathBuf {
    let standard = standardized(path);
    let mut existing = standard.as_path();
    let mut missing = Vec::new();
    loop {
        if let Ok(mut resolved) = std::fs::canonicalize(existing) {
            resolved.extend(missing.iter().rev());
            return resolved;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_owned());
                existing = parent;
            }
            _ => return standard,
        }
    }
}

fn standardized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => out.push(component),
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                }
            }
        }
    }
    out
}

/// The MIME type of `path` by its extension, as upstream's `UTType` lookup (also the type a `WebView2` page's granted
/// file is served with).
#[must_use]
pub fn mime_type(path: &Path) -> String {
    mime_guess::from_path(path)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_owned()
}

// MARK: - the stream

/// A page channel's connection as the frontend hands it over: shareable with the threads that serve files and send
/// replies. Lines go out without their newline.
pub trait PageChannel: Send + Sync {
    /// Queues one line; false when the connection is closing or its queue is full. Never blocks.
    fn send(&self, line: Vec<u8>) -> bool;
    /// Queues one line, waiting while the queue is full; false once the connection is closing. It blocks its thread,
    /// so only a producer off the main loop uses it, one that must not outrun the peer.
    fn send_waiting(&self, line: Vec<u8>) -> bool;
    /// Closes the connection; idempotent.
    fn shutdown(&self);
}

/// Counts one kind of request a channel took in and has not finished (upstream's `PageServeAdmission`). It never
/// blocks: a request past the limit is refused, so the reader keeps reading the cancels and the close behind it.
#[derive(Debug)]
struct Admission {
    limit: usize,
    taken: AtomicUsize,
}

impl Admission {
    const fn new(limit: usize) -> Self {
        Self {
            limit,
            taken: AtomicUsize::new(0),
        }
    }

    /// Takes a slot; false when all are taken.
    fn try_acquire(&self) -> bool {
        self.taken
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |taken| {
                (taken < self.limit).then_some(taken + 1)
            })
            .is_ok()
    }

    fn release(&self) {
        let released = self
            .taken
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |taken| taken.checked_sub(1));
        if released.is_err() {
            tracing::error!("page channel: a slot released twice");
        }
    }
}

/// The transfers taken in, and those the presenter cancelled since.
#[derive(Debug, Default)]
struct Transfers {
    active: HashSet<i64>,
    cancelled: HashSet<i64>,
}

/// The queues of the threads behind a stream: one reads files, one writes bridge replies, so a reply waits behind at
/// most the chunks already queued for the writer, never behind a whole file.
#[derive(Default)]
struct Workers {
    files: Option<Sender<(i64, String)>>,
    replies: Option<Sender<PageServeFrame>>,
}

/// The channel of one page this machine handed to the presenter (upstream's `PageServeStream`): it serves the page's
/// files from inside its grant and runs the page's bridge requests here.
pub struct PageServeStream {
    page: Uuid,
    source: HtmlSource,
    channel: Arc<dyn PageChannel>,
    resources: Admission,
    bridges: Admission,
    transfers: Mutex<Transfers>,
    workers: Mutex<Workers>,
    /// Set once the page's end is under way, by whichever side started it, so it is not ended twice.
    ending: AtomicBool,
}

impl std::fmt::Debug for PageServeStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageServeStream")
            .field("page", &self.page)
            .field("ending", &self.ending)
            .finish_non_exhaustive()
    }
}

/// What a line read on a channel's thread leaves for the main loop: a bridge call to run, or the page to end.
#[derive(Debug)]
pub struct PageServeWork {
    stream: Arc<PageServeStream>,
    task: Task,
}

#[derive(Debug)]
enum Task {
    Bridge { id: i64, request: String },
    Fail(&'static str),
}

/// A page's bridge request ready to run: the frontend dispatches `request` in the same main-loop turn and hands the
/// response to `reply`.
#[derive(Debug)]
pub struct PageServeCall {
    pub request: Request,
    pub reply: BridgeReply,
}

/// Where a bridge request's response goes: the channel, as a `reply` frame, off the main loop. Dropped unanswered, it
/// answers [`CONTROL_UNAVAILABLE`], so the call's slot is always freed.
#[derive(Debug)]
pub struct BridgeReply {
    stream: Arc<PageServeStream>,
    id: i64,
    answered: bool,
}

impl BridgeReply {
    /// Sends `response`, raw, as the request's reply.
    pub fn answer(mut self, response: &Response) {
        self.answered = true;
        let frame = match serde_json::to_string(response) {
            Ok(response) => PageServeFrame::Reply { id: self.id, response },
            Err(_) => failure(self.id, UNENCODABLE),
        };
        self.stream.reply(frame);
    }
}

impl Drop for BridgeReply {
    fn drop(&mut self) {
        if !self.answered {
            self.stream.reply(failure(self.id, CONTROL_UNAVAILABLE));
        }
    }
}

fn failure(id: i64, error: &str) -> PageServeFrame {
    PageServeFrame::Failure {
        id,
        error: error.to_owned(),
    }
}

impl PageServeStream {
    /// A stream for `page`, whose source decides what it serves, over `channel`, with its worker threads started.
    ///
    /// # Errors
    ///
    /// A worker thread could not be started; the channel is shut then.
    pub(crate) fn start(page: Uuid, source: HtmlSource, channel: Arc<dyn PageChannel>) -> std::io::Result<Arc<Self>> {
        let stream = Arc::new(Self {
            page,
            source,
            channel,
            resources: Admission::new(RESOURCE_LIMIT),
            bridges: Admission::new(BRIDGE_LIMIT),
            transfers: Mutex::new(Transfers::default()),
            workers: Mutex::new(Workers::default()),
            ending: AtomicBool::new(false),
        });
        let (files, file_jobs) = mpsc::channel();
        let (replies, reply_jobs) = mpsc::channel();
        let file_stream = Arc::clone(&stream);
        let reply_stream = Arc::clone(&stream);
        let spawned = thread::Builder::new()
            .name("ngterm-page-serve-files".to_owned())
            .spawn(move || file_stream.serve_files(&file_jobs))
            .and_then(|_| {
                thread::Builder::new()
                    .name("ngterm-page-serve-replies".to_owned())
                    .spawn(move || reply_stream.send_replies(&reply_jobs))
            });
        if let Err(error) = spawned {
            // the file thread, if it started, ends with its queue's sender
            drop(files);
            stream.channel.shutdown();
            return Err(error);
        }
        *stream.lock_workers() = Workers {
            files: Some(files),
            replies: Some(replies),
        };
        Ok(stream)
    }

    /// The page this channel serves.
    #[must_use]
    pub fn page(&self) -> Uuid {
        self.page
    }

    fn lock_transfers(&self) -> MutexGuard<'_, Transfers> {
        self.transfers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_workers(&self) -> MutexGuard<'_, Workers> {
        self.workers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Handles one line on the connection's thread (upstream's `take`): a cancel applies at once, a resource request
    /// takes a slot and goes to the file thread, a bridge request takes a slot and goes to the main loop, a request
    /// past its limit is refused at once by id, and anything else ends the page. Nothing here waits, so the cancels
    /// and the close behind a request are always read.
    pub fn take(self: &Arc<Self>, line: &[u8]) -> Option<PageServeWork> {
        let Ok(frame) = decode(line) else {
            return Some(self.work(Task::Fail("a malformed frame")));
        };
        match frame {
            PageServeFrame::Cancel { id } => {
                let mut transfers = self.lock_transfers();
                if transfers.active.contains(&id) {
                    transfers.cancelled.insert(id);
                }
                None
            }
            PageServeFrame::Resource { id, path } => {
                if !self.resources.try_acquire() {
                    return self.refuse(id);
                }
                self.lock_transfers().active.insert(id);
                let queued = self
                    .lock_workers()
                    .files
                    .as_ref()
                    .is_some_and(|files| files.send((id, path)).is_ok());
                if !queued {
                    self.finish_transfer(id);
                }
                None
            }
            PageServeFrame::Bridge { id, request } => {
                if !self.bridges.try_acquire() {
                    return self.refuse(id);
                }
                Some(self.work(Task::Bridge { id, request }))
            }
            PageServeFrame::Head { .. }
            | PageServeFrame::Chunk { .. }
            | PageServeFrame::End { .. }
            | PageServeFrame::Failure { .. }
            | PageServeFrame::Reply { .. } => Some(self.work(Task::Fail("a frame only the origin sends"))),
        }
    }

    fn work(self: &Arc<Self>, task: Task) -> PageServeWork {
        PageServeWork {
            stream: Arc::clone(self),
            task,
        }
    }

    /// Refuses request `id` as busy. A helper past the limit while the writer's queue is full is not reading what it
    /// asked for, so the page ends rather than the reader waiting or the refusals queueing without bound.
    fn refuse(self: &Arc<Self>, id: i64) -> Option<PageServeWork> {
        let sent = encode(&failure(id, BUSY)).is_ok_and(|line| self.channel.send(without_newline(line)));
        (!sent).then(|| self.work(Task::Fail("more requests than it reads answers to")))
    }

    /// Closes the channel without ending the page, which the caller already did or is doing; the worker threads stop
    /// once their queues drain.
    pub fn shutdown(&self) {
        self.ending.store(true, Ordering::SeqCst);
        self.channel.shutdown();
        self.stop_workers();
    }

    fn stop_workers(&self) {
        let workers = std::mem::take(&mut *self.lock_workers());
        drop(workers);
    }

    /// Queues `frame` for the reply thread, which frees the bridge slot once it is written; with the channel closed
    /// the slot is freed at once.
    fn reply(&self, frame: PageServeFrame) {
        let queued = self
            .lock_workers()
            .replies
            .as_ref()
            .is_some_and(|replies| replies.send(frame).is_ok());
        if !queued {
            self.bridges.release();
        }
    }

    fn send_replies(&self, jobs: &Receiver<PageServeFrame>) {
        for frame in jobs {
            self.send_waiting(&frame);
            self.bridges.release();
        }
    }

    fn serve_files(&self, jobs: &Receiver<(i64, String)>) {
        for (id, path) in jobs {
            self.transfer(id, &path);
            self.finish_transfer(id);
        }
    }

    fn finish_transfer(&self, id: i64) {
        let mut transfers = self.lock_transfers();
        transfers.active.remove(&id);
        transfers.cancelled.remove(&id);
        drop(transfers);
        self.resources.release();
    }

    fn cancelled(&self, id: i64) -> bool {
        self.lock_transfers().cancelled.contains(&id)
    }

    /// Serves one resource. Every admitted transfer ends in exactly one terminal frame, a cancelled one too, so the
    /// presenter frees its request slot only once this side has.
    fn transfer(&self, id: i64, path: &str) {
        if self.cancelled(id) {
            self.send_waiting(&failure(id, CANCELLED));
            return;
        }
        let Some(file) = resolve(path, &self.source) else {
            self.send_waiting(&failure(id, OUTSIDE_FOLDER));
            return;
        };
        let opened = File::open(&file).and_then(|handle| Ok((handle.metadata()?.is_file(), handle)));
        let Ok((true, mut handle)) = opened else {
            self.send_waiting(&failure(id, UNREADABLE));
            return;
        };
        let head = PageServeFrame::Head {
            id,
            mime_type: mime_type(&file),
        };
        if !self.send_waiting(&head) {
            return;
        }
        while !self.cancelled(id) {
            let mut data = Vec::with_capacity(MAX_CHUNK_BYTES);
            match (&mut handle).take(MAX_CHUNK_BYTES as u64).read_to_end(&mut data) {
                Err(_) => {
                    self.send_waiting(&failure(id, UNREADABLE));
                    return;
                }
                Ok(0) => {
                    self.send_waiting(&PageServeFrame::End { id });
                    return;
                }
                Ok(_) => {
                    if !self.send_waiting(&PageServeFrame::Chunk { id, data }) {
                        return;
                    }
                }
            }
        }
        self.send_waiting(&failure(id, CANCELLED));
    }

    /// Sends `frame`, waiting for room; false once the channel is closing. A frame too large for one line goes back as
    /// a failure, so the page learns why and the channel stays.
    fn send_waiting(&self, frame: &PageServeFrame) -> bool {
        let line = encode(frame).or_else(|_| encode(&failure(frame.id(), ANSWER_TOO_LARGE)));
        line.is_ok_and(|line| self.channel.send_waiting(without_newline(line)))
    }
}

fn without_newline(mut line: Vec<u8>) -> Vec<u8> {
    line.pop();
    line
}

// MARK: - the origin's channels

/// The pages whose claim was answered and the channels adopted for them (upstream's `pendingPageClaims` and
/// `pageServeStreams`), and the claim expiries the frontend owes.
#[derive(Debug, Default)]
pub(crate) struct PageServeRuntime {
    /// Pages whose claim was answered `ok` and whose connection is not adopted yet, with the source it serves.
    pending_claims: HashMap<Uuid, HtmlSource>,
    streams: HashMap<Uuid, Arc<PageServeStream>>,
    /// Claim expiry runs the frontend owes, each after its delay.
    pub(super) expiries: Vec<Duration>,
}

impl PageServeRuntime {
    /// Records the claim of `page` answered `ok`.
    pub(crate) fn claimed(&mut self, page: Uuid, source: HtmlSource) {
        self.pending_claims.insert(page, source);
    }
}

impl Core {
    /// Takes over the connection of a page whose claim was just answered `ok` (upstream's `adoptPageServeStream`).
    /// A page that ended between its claim and now gets no channel: its connection is shut and `None` comes back.
    pub fn adopt_page_serve_stream(
        &mut self,
        page: &str,
        channel: Arc<dyn PageChannel>,
    ) -> Option<Arc<PageServeStream>> {
        let page = frames::parse_uuid(page);
        let source = page.and_then(|page| self.remote.page_serve.pending_claims.remove(&page));
        let (Some(page), Some(source)) = (page, source) else {
            channel.shutdown();
            return None;
        };
        if self.remote_page_place(page).is_none() {
            channel.shutdown();
            return None;
        }
        match PageServeStream::start(page, source, channel) {
            Ok(stream) => {
                self.remote.page_serve.streams.insert(page, Arc::clone(&stream));
                Some(stream)
            }
            Err(error) => {
                tracing::error!(%error, "ending a page whose channel could not start");
                self.end_remote_page_anywhere(page);
                None
            }
        }
    }

    /// Runs what a channel's thread left for the main loop: a bridge request against the page where it sits now,
    /// handed back to run in this same turn (so one queued before its page closed or moved never runs against the old
    /// place), or the page's end.
    pub fn page_serve_work(&mut self, work: PageServeWork) -> Option<PageServeCall> {
        let PageServeWork { stream, task } = work;
        match task {
            Task::Fail(reason) => {
                self.fail_page_serve(&stream, reason);
                None
            }
            Task::Bridge { id, request } => self.page_serve_bridge(stream, id, &request),
        }
    }

    fn page_serve_bridge(&mut self, stream: Arc<PageServeStream>, id: i64, request: &str) -> Option<PageServeCall> {
        let Some(place) = self.remote_page_place(stream.page) else {
            stream.reply(failure(id, PAGE_CLOSED));
            return None;
        };
        let origin = HtmlBridgePage {
            window: Some(uuid_string(place.window)),
            session: place.session,
            pane: place.pane,
        };
        match bridge::request(request.as_bytes(), &origin) {
            Ok(request) => Some(PageServeCall {
                request,
                reply: BridgeReply {
                    stream,
                    id,
                    answered: false,
                },
            }),
            Err(refusal) => {
                stream.reply(failure(id, &refusal.message));
                None
            }
        }
    }

    /// A channel sent what a helper never sends, or more than it reads: its page ends, the presenter stays.
    fn fail_page_serve(&mut self, stream: &PageServeStream, reason: &str) {
        if stream.ending.swap(true, Ordering::SeqCst) {
            return;
        }
        tracing::error!(reason, "ending a page whose channel misbehaved");
        self.end_remote_page_anywhere(stream.page);
        stream.shutdown();
    }

    /// A channel's connection closed (upstream's `PageServeStream.closed`): a page whose end was not already under way
    /// ends, as nobody is left to load it. A claim whose reply never reached the helper closes this way too.
    pub fn page_serve_closed(&mut self, stream: &Arc<PageServeStream>) {
        let streams = &mut self.remote.page_serve.streams;
        if streams.get(&stream.page).is_some_and(|held| Arc::ptr_eq(held, stream)) {
            streams.remove(&stream.page);
        }
        stream.stop_workers();
        if stream.ending.load(Ordering::SeqCst) {
            return;
        }
        self.end_remote_page_anywhere(stream.page);
    }

    /// A page handed to the presenter ended: its channel, if one was adopted, is shut once, and a claim not adopted yet
    /// is forgotten (upstream's `RemotePageEnds.onEnd`).
    pub(super) fn shut_page_serve_channel(&mut self, page: Uuid) {
        self.remote.page_serve.pending_claims.remove(&page);
        if let Some(stream) = self.remote.page_serve.streams.remove(&page) {
            stream.shutdown();
        }
    }

    /// The claim expiry runs armed since the last take, each after its delay (upstream's `schedulePageClaimExpiry`).
    pub fn take_page_claim_expiries(&mut self) -> Vec<Duration> {
        std::mem::take(&mut self.remote.page_serve.expiries)
    }

    /// Whether a channel is adopted for `page`.
    #[must_use]
    pub fn page_serve_adopted(&self, page: Uuid) -> bool {
        self.remote.page_serve.streams.contains_key(&page)
    }
}

#[cfg(test)]
#[path = "page_serve_tests.rs"]
mod tests;
