//! The page-serve channel on the origin (upstream's `ControlDispatcherOverlayTests` page-serve cases,
//! `AppStoreRemotePageTests`' claim and expiry cases and `ControlServerPageServeTests`' channel cases): the claim, the
//! adoption, the file transfers, the bridge calls, and every way a channel or its page ends. The connection is a fake
//! with the stream's bounded writer queue; the test reads it as the helper would.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Command, Request};
use serde_json::json;
use uuid::Uuid;

use super::remote_page::Origin;
use super::{args, failure, wire};
use crate::html::HtmlSource;
use crate::remote::PAGE_CLAIM_WINDOW;
use crate::remote::page_serve::{
    BUSY, CANCELLED, CONTROL_UNAVAILABLE, MAX_PENDING_LINES, PageChannel, PageServeFrame, PageServeStream,
    RESOURCE_LIMIT, decode, encode,
};

/// How long a test waits for the file or reply thread before it fails.
const PATIENCE: Duration = Duration::from_secs(10);
/// Bytes of the file a transfer cannot finish before the test reads it.
const HUGE: usize = 8 * 1024 * 1024;

#[derive(Default)]
struct Queue {
    lines: VecDeque<Vec<u8>>,
    closed: bool,
    shutdowns: usize,
}

/// A connection with the stream's writer queue: lines wait in it until the test reads them, as they would wait for a
/// helper that has not read them yet.
struct FakeChannel {
    capacity: usize,
    queue: Mutex<Queue>,
    changed: Condvar,
}

impl FakeChannel {
    fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            capacity,
            queue: Mutex::new(Queue::default()),
            changed: Condvar::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The next frame, waiting for one; `None` once the channel is closed with nothing left to read.
    fn frame(&self) -> Option<PageServeFrame> {
        let deadline = Instant::now() + PATIENCE;
        let mut queue = self.lock();
        loop {
            if let Some(line) = queue.lines.pop_front() {
                self.changed.notify_all();
                return Some(decode(&line).unwrap());
            }
            assert!(Instant::now() < deadline, "no frame in time");
            if queue.closed {
                return None;
            }
            queue = self.changed.wait_timeout(queue, PATIENCE).unwrap().0;
        }
    }

    /// The frames of request `id` up to its terminal one: its MIME type, body and failure.
    fn body(&self, id: i64) -> (Option<String>, Vec<u8>, Option<String>) {
        let (mut mime, mut data) = (None, Vec::new());
        while let Some(frame) = self.frame() {
            match frame {
                PageServeFrame::Head { id: of, mime_type } if of == id => mime = Some(mime_type),
                PageServeFrame::Chunk { id: of, data: bytes } if of == id => data.extend(bytes),
                PageServeFrame::End { id: of } if of == id => return (mime, data, None),
                PageServeFrame::Failure { id: of, error } if of == id => return (mime, data, Some(error)),
                _ => {}
            }
        }
        (mime, data, Some("channel closed".to_owned()))
    }

    /// Waits until the writer queue holds `count` lines.
    fn wait_queued(&self, count: usize) {
        let deadline = Instant::now() + PATIENCE;
        let mut queue = self.lock();
        while queue.lines.len() < count {
            assert!(Instant::now() < deadline, "the queue never filled");
            queue = self.changed.wait_timeout(queue, PATIENCE).unwrap().0;
        }
    }

    fn closed(&self) -> bool {
        self.lock().closed
    }

    fn shutdowns(&self) -> usize {
        self.lock().shutdowns
    }
}

impl PageChannel for FakeChannel {
    fn send(&self, line: Vec<u8>) -> bool {
        let mut queue = self.lock();
        if queue.closed || queue.lines.len() >= self.capacity {
            return false;
        }
        queue.lines.push_back(line);
        self.changed.notify_all();
        true
    }

    fn send_waiting(&self, line: Vec<u8>) -> bool {
        let mut queue = self.lock();
        while !queue.closed && queue.lines.len() >= self.capacity {
            queue = self.changed.wait(queue).unwrap_or_else(PoisonError::into_inner);
        }
        if queue.closed {
            return false;
        }
        queue.lines.push_back(line);
        self.changed.notify_all();
        true
    }

    fn shutdown(&self) {
        let mut queue = self.lock();
        queue.closed = true;
        queue.shutdowns += 1;
        self.changed.notify_all();
    }
}

/// A presented session with a folder of page files, and a page handed over from it.
struct Served {
    origin: Origin,
    _dir: tempfile::TempDir,
    docs: PathBuf,
}

impl Served {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let docs = std::fs::canonicalize(dir.path()).unwrap().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("index.html"), "<p>index</p>").unwrap();
        Self {
            origin: Origin::pages(),
            _dir: dir,
            docs,
        }
    }

    fn path(&self, name: &str) -> String {
        text(&self.docs.join(name))
    }

    /// Opens the folder's index as a file page (granted the folder with `grant`); its page id.
    fn open_file(&mut self, grant: bool) -> String {
        let (index, docs) = (self.path("index.html"), text(&self.docs));
        let response = self.origin.overlay(SessionCmd::OverlayOpen, |a| {
            a.html = Some(index);
            a.cwd = grant.then_some(docs);
            a.javascript = Some(true);
        });
        assert!(response.ok, "{response:?}");
        response.result.unwrap()["pageID"].as_str().unwrap().to_owned()
    }

    /// The claim token the presenter got with the last page.
    fn token(&self) -> String {
        let request = self.origin.presenter.last("page.request").unwrap();
        request["page"]["token"].as_str().unwrap().to_owned()
    }

    fn claim(&mut self, token: &str) -> ngterm_proto::Response {
        let request = Request {
            target: Some(token.to_owned()),
            ..Request::new(Command::Session(SessionCmd::OverlayPageServe))
        };
        self.origin.fixture().core.dispatch(&request)
    }

    /// Claims the last page's channel and adopts a fake connection for it, as the frontend does after an `ok`.
    fn channel(&mut self, page: &str) -> Channel {
        self.channel_with(page, MAX_PENDING_LINES)
    }

    fn channel_with(&mut self, page: &str, capacity: usize) -> Channel {
        let token = self.token();
        let claimed = self.claim(&token);
        assert_eq!(claimed.result, Some(json!({"id": page})), "{claimed:?}");
        let fake = FakeChannel::new(capacity);
        let shared: Arc<dyn PageChannel> = fake.clone();
        let stream = self
            .origin
            .fixture()
            .core
            .adopt_page_serve_stream(page, shared)
            .unwrap();
        Channel { fake, stream }
    }

    /// Hands `line` to the channel as its reader would, and runs what it leaves for the main loop.
    fn take(&mut self, channel: &Channel, line: &[u8]) {
        if let Some(work) = channel.stream.take(line) {
            let core = &mut self.origin.fixture().core;
            if let Some(call) = core.page_serve_work(work) {
                let response = core.dispatch(&call.request);
                call.reply.answer(&response);
            }
            core.commit();
        }
    }

    fn send(&mut self, channel: &Channel, frame: &PageServeFrame) {
        let mut line = encode(frame).unwrap();
        line.pop();
        self.take(channel, &line);
    }

    /// The connection closed, as the frontend reports it.
    fn close(&mut self, channel: &Channel) {
        let core = &mut self.origin.fixture().core;
        core.page_serve_closed(&channel.stream);
        core.commit();
    }

    /// The pages the presenter was told to close, in order.
    fn page_closes(&self) -> Vec<serde_json::Value> {
        self.origin
            .presenter
            .frames("page.close")
            .into_iter()
            .map(|frame| frame["page"]["page"].clone())
            .collect()
    }

    fn presenter_kept(&mut self) -> bool {
        let session = self.origin.session();
        self.origin.fixture().core.remote.hub.has_presenter(session)
    }
}

struct Channel {
    fake: Arc<FakeChannel>,
    stream: Arc<PageServeStream>,
}

fn text(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

fn resource(id: i64, path: &str) -> PageServeFrame {
    PageServeFrame::Resource {
        id,
        path: path.to_owned(),
    }
}

fn bridge(id: i64, request: &str) -> PageServeFrame {
    PageServeFrame::Bridge {
        id,
        request: request.to_owned(),
    }
}

fn page_id(page: &str) -> Uuid {
    Uuid::parse_str(page).unwrap()
}

// MARK: - the claim

/// Upstream's `pageServeRejectsAMissingOrMalformedTokenBeforeCallingActions`, and an unknown token.
#[test]
fn a_missing_malformed_or_unknown_token_is_refused() {
    let mut served = Served::new();
    let core = &mut served.origin.fixture().core;
    let cmd = Command::Session(SessionCmd::OverlayPageServe);
    for (target, error) in [
        (None, "session.overlay.page.serve requires a page token"),
        (Some("  "), "session.overlay.page.serve requires a page token"),
        (Some("not-a-uuid"), "invalid page token"),
        (Some("00000000-0000-0000-0000-000000000000"), "page not claimable"),
    ] {
        let request = Request {
            target: target.map(str::to_owned),
            ..Request::new(cmd)
        };
        assert_eq!(core.dispatch(&request), failure(error), "{target:?}");
    }
}

/// Upstream's `aTokenClaimsOnceWithinItsDeadlineForTheSamePresenter` and `testATokenClaimsOnce`.
#[test]
fn a_token_claims_once_within_its_deadline() {
    let mut served = Served::new();
    let page = served.open_file(true);
    let token = served.token();
    let core = &mut served.origin.fixture().core;
    let soon = Instant::now() + Duration::from_secs(5);
    assert_eq!(core.claim_remote_page(&wire(Uuid::new_v4()), soon), None);
    let (claimed, source) = core.claim_remote_page(&token, soon).unwrap();
    assert_eq!(claimed, page_id(&page));
    assert!(matches!(
        source,
        HtmlSource::File {
            grant_root: Some(_),
            ..
        }
    ));
    assert_eq!(core.claim_remote_page(&token, soon), None);
    assert_eq!(served.claim(&token), failure("page not claimable"));
}

/// Upstream's `aLateClaimIsRefusedAndExpiryEndsOnlyAnUnclaimedFilePage`.
#[test]
fn a_late_claim_is_refused_and_expiry_ends_only_an_unclaimed_file_page() {
    let mut origin = Origin::new(true, &crate::remote::hub::SUPPORTED_KINDS);
    let file = origin.opened("/tmp/docs/index.html", |_| {});
    let token = origin.presenter.last("page.request").unwrap()["page"]["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let web = origin.opened("https://example.com/", |a| a.pane = Some("left".to_owned()));
    let core = &mut origin.fixture().core;
    assert_eq!(
        core.take_page_claim_expiries(),
        [PAGE_CLAIM_WINDOW + Duration::from_millis(100)],
        "armed for the file page only"
    );
    let late = Instant::now() + PAGE_CLAIM_WINDOW + Duration::from_secs(1);
    assert_eq!(core.claim_remote_page(&token, late), None);
    core.expire_remote_page_claims_at(late);
    core.commit();
    assert_eq!(origin.slot_jobs(), [web]);
    assert_eq!(origin.outcome(&file)["outcome"], "dismissed");
    assert_eq!(origin.presenter.frames("page.close").len(), 1);
}

#[test]
fn a_claimed_page_does_not_expire() {
    let mut served = Served::new();
    let page = served.open_file(true);
    let _channel = served.channel(&page);
    let core = &mut served.origin.fixture().core;
    core.expire_remote_page_claims_at(Instant::now() + PAGE_CLAIM_WINDOW + Duration::from_secs(1));
    core.commit();
    assert_eq!(served.origin.slot_jobs(), [page]);
}

/// A page that ended between its claim and the adoption gets no channel.
#[test]
fn a_page_that_ended_before_its_adoption_has_its_connection_shut() {
    let mut served = Served::new();
    let page = served.open_file(true);
    let token = served.token();
    assert!(served.claim(&token).ok);
    assert!(served.origin.overlay(SessionCmd::OverlayClose, |_| {}).ok);
    let fake = FakeChannel::new(MAX_PENDING_LINES);
    let core = &mut served.origin.fixture().core;
    assert!(core.adopt_page_serve_stream(&page, fake.clone()).is_none());
    assert!(fake.closed());
    assert!(!core.page_serve_adopted(page_id(&page)));
}

// MARK: - resources

/// Upstream's `testAClaimedChannelServesThePageAndAFileInsideItsGrant`.
#[test]
fn a_claimed_channel_serves_the_page_and_a_file_inside_its_grant() {
    let mut served = Served::new();
    let big: Vec<u8> = (0..200_000_u32).map(|n| u8::try_from(n % 251).unwrap()).collect();
    std::fs::write(served.docs.join("big.bin"), &big).unwrap();
    let page = served.open_file(true);
    let channel = served.channel(&page);
    let (index, big_path) = (served.path("index.html"), served.path("big.bin"));
    served.send(&channel, &resource(1, &index));
    assert_eq!(
        channel.fake.body(1),
        (Some("text/html".to_owned()), b"<p>index</p>".to_vec(), None)
    );
    served.send(&channel, &resource(2, &big_path));
    let (_, data, error) = channel.fake.body(2);
    assert_eq!(error, None);
    assert_eq!(data, big);
}

/// Upstream's `testAFileOutsideTheGrantIsRefusedOnTheChannel` and `testAFileThatFailsToReadAnswersAFailure`.
#[test]
fn a_file_outside_the_grant_or_unreadable_answers_a_failure() {
    let mut served = Served::new();
    std::fs::create_dir(served.docs.join("folder")).unwrap();
    let page = served.open_file(true);
    let channel = served.channel(&page);
    let outside = text(&served.docs.parent().unwrap().join("elsewhere.txt"));
    served.send(&channel, &resource(1, &outside));
    assert_eq!(
        channel.fake.body(1).2.as_deref(),
        Some("file is outside the page's folder")
    );
    let folder = served.path("folder");
    served.send(&channel, &resource(2, &folder));
    assert_eq!(
        channel.fake.body(2),
        (None, Vec::new(), Some("file cannot be read".to_owned()))
    );
    let missing = served.path("missing.css");
    served.send(&channel, &resource(3, &missing));
    assert_eq!(channel.fake.body(3).2.as_deref(), Some("file cannot be read"));
}

#[test]
fn a_page_without_a_grant_serves_only_its_own_file() {
    let mut served = Served::new();
    std::fs::write(served.docs.join("style.css"), "p {}").unwrap();
    let page = served.open_file(false);
    let channel = served.channel(&page);
    let (index, style) = (served.path("index.html"), served.path("style.css"));
    served.send(&channel, &resource(1, &index));
    assert_eq!(channel.fake.body(1).1, b"<p>index</p>");
    served.send(&channel, &resource(2, &style));
    assert_eq!(
        channel.fake.body(2).2.as_deref(),
        Some("file is outside the page's folder")
    );
}

/// Upstream's `testACancelledTransferStopsAndTheNextRequestIsServed`.
#[test]
fn a_cancelled_transfer_stops_and_the_next_request_is_served() {
    let mut served = Served::new();
    std::fs::write(served.docs.join("huge.bin"), vec![0_u8; HUGE]).unwrap();
    let page = served.open_file(true);
    let channel = served.channel(&page);
    let (huge, index) = (served.path("huge.bin"), served.path("index.html"));
    served.send(&channel, &resource(1, &huge));
    served.send(&channel, &PageServeFrame::Cancel { id: 1 });
    served.send(&channel, &resource(2, &index));
    let (mut huge_ended, mut huge_cancelled, mut data) = (false, false, Vec::new());
    while let Some(frame) = channel.fake.frame() {
        match frame {
            PageServeFrame::End { id: 1 } => huge_ended = true,
            PageServeFrame::Failure { id: 1, error } => huge_cancelled = error == CANCELLED,
            PageServeFrame::Chunk { id: 2, data: bytes } => data.extend(bytes),
            PageServeFrame::End { id: 2 } => break,
            _ => {}
        }
    }
    assert!(!huge_ended);
    assert!(huge_cancelled, "a cancelled transfer still ends in one terminal frame");
    assert_eq!(data, b"<p>index</p>");
}

/// Upstream's `testASaturatedChannelStillServesCancelsAndBridgeCalls`.
#[test]
fn a_saturated_channel_still_serves_cancels_and_bridge_calls() {
    let mut served = Served::new();
    std::fs::write(served.docs.join("huge.bin"), vec![0_u8; HUGE]).unwrap();
    let page = served.open_file(true);
    let channel = served.channel(&page);
    let huge = served.path("huge.bin");
    let limit = i64::try_from(RESOURCE_LIMIT).unwrap();
    for id in 1..=limit {
        served.send(&channel, &resource(id, &huge));
    }
    for id in 1..=limit {
        served.send(&channel, &PageServeFrame::Cancel { id });
    }
    served.send(&channel, &bridge(100, r#"{"cmd":"version"}"#));
    let (mut failures, mut ends, mut replied) = (std::collections::HashMap::new(), 0, false);
    while failures.len() < RESOURCE_LIMIT || !replied {
        match channel.fake.frame().unwrap() {
            PageServeFrame::Failure { id, error } => {
                failures.insert(id, error);
            }
            PageServeFrame::End { .. } => ends += 1,
            PageServeFrame::Reply { id: 100, response } => {
                replied = serde_json::from_str::<serde_json::Value>(&response).unwrap()["ok"] == true;
            }
            _ => {}
        }
    }
    for id in 1..=limit {
        assert_eq!(failures[&id], CANCELLED, "transfer {id}");
    }
    assert_eq!(ends, 0);
    assert_eq!(served.origin.slot_jobs(), [page]);
    assert!(served.presenter_kept());
}

/// Upstream's `testOverloadWithAFullWriterQueueEndsThePageWithoutWaiting`.
#[test]
fn overload_with_a_full_writer_queue_ends_the_page_without_waiting() {
    let mut served = Served::new();
    std::fs::write(served.docs.join("huge.bin"), vec![0_u8; HUGE]).unwrap();
    let page = served.open_file(true);
    let channel = served.channel(&page);
    let huge = served.path("huge.bin");
    served.send(&channel, &resource(1, &huge));
    channel.fake.wait_queued(MAX_PENDING_LINES);
    let started = Instant::now();
    for id in 2..=i64::try_from(RESOURCE_LIMIT).unwrap() + 1 {
        served.send(&channel, &resource(id, &huge));
    }
    assert!(served.origin.slot_jobs().is_empty());
    assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
    assert!(channel.fake.closed());
    assert_eq!(served.page_closes().len(), 1);
    assert!(served.presenter_kept());
}

/// Upstream's `testShuttingDownWithPipelinedRequestsClosesTheChannel`: refusals queue up while the queue has room, the
/// shutdown stops every transfer, and the page stays.
#[test]
fn shutting_down_with_pipelined_requests_closes_the_channel() {
    let mut served = Served::new();
    std::fs::write(served.docs.join("huge.bin"), vec![0_u8; HUGE]).unwrap();
    let page = served.open_file(true);
    let channel = served.channel_with(&page, 64);
    let huge = served.path("huge.bin");
    for id in 1..=i64::try_from(RESOURCE_LIMIT * 3).unwrap() {
        served.send(&channel, &resource(id, &huge));
    }
    let busy = channel
        .fake
        .lock()
        .lines
        .iter()
        .filter(|line| matches!(decode(line), Ok(PageServeFrame::Failure { error, .. }) if error == BUSY))
        .count();
    assert_eq!(busy, RESOURCE_LIMIT * 2);
    channel.stream.shutdown();
    served.close(&channel);
    let deadline = Instant::now() + PATIENCE;
    while Arc::strong_count(&channel.stream) > 1 {
        assert!(Instant::now() < deadline, "the channel's threads never finished");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!served.origin.fixture().core.page_serve_adopted(page_id(&page)));
    assert_eq!(served.origin.slot_jobs(), [page]);
}

// MARK: - bridge calls

/// Upstream's `testARelayedSubmitIsRecordedOnThisMac`.
#[test]
fn a_relayed_submit_is_recorded_here_and_closes_the_page_and_its_channel() {
    let mut served = Served::new();
    let page = served.open_file(true);
    let channel = served.channel(&page);
    served.send(
        &channel,
        &bridge(7, r#"{"cmd":"session.overlay.submit","args":{"value":"notes"}}"#),
    );
    assert_eq!(
        channel.fake.frame(),
        None,
        "the submit closes the page, and its channel with it"
    );
    assert_eq!(channel.fake.shutdowns(), 1, "shut once");
    assert_eq!(served.origin.outcome(&page)["value"], "notes");
    assert_eq!(served.page_closes().len(), 1);
    assert!(served.origin.slot_jobs().is_empty());
    assert!(!served.origin.fixture().core.page_serve_adopted(page_id(&page)));
}

#[test]
fn a_bridge_reply_is_the_raw_response_and_a_refusal_comes_back_as_a_failure() {
    let mut served = Served::new();
    let page = served.open_file(true);
    let channel = served.channel(&page);
    served.send(&channel, &bridge(1, r#"{"cmd":"session.overlay.result"}"#));
    let Some(PageServeFrame::Reply { id: 1, response }) = channel.fake.frame() else {
        panic!("no reply");
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&response).unwrap(),
        json!({"ok": false, "error": "no overlay result: the slot holds an html page"}),
        "the page's own session filled in"
    );
    served.send(&channel, &bridge(2, r#"{"cmd":"session.overlay.page.serve"}"#));
    assert_eq!(
        channel.fake.frame(),
        Some(PageServeFrame::Failure {
            id: 2,
            error: "session.overlay.page.serve cannot be sent from a page".to_owned()
        })
    );
    served.send(&channel, &bridge(3, "not json"));
    assert_eq!(
        channel.fake.frame(),
        Some(PageServeFrame::Failure {
            id: 3,
            error: "invalid request: The given data was not valid JSON.".to_owned()
        })
    );
}

#[test]
fn a_call_the_frontend_drops_answers_control_unavailable() {
    let mut served = Served::new();
    let page = served.open_file(true);
    let channel = served.channel(&page);
    let work = channel
        .stream
        .take(br#"{"id":4,"kind":"bridge","request":"{\"cmd\":\"version\"}"}"#);
    let call = served.origin.fixture().core.page_serve_work(work.unwrap()).unwrap();
    assert_eq!(call.request.cmd, Command::Version);
    drop(call);
    assert_eq!(
        channel.fake.frame(),
        Some(PageServeFrame::Failure {
            id: 4,
            error: CONTROL_UNAVAILABLE.to_owned()
        })
    );
}

/// Upstream's `testABridgeRequestForAPageThatEndedIsRefused`.
#[test]
fn a_bridge_request_for_a_page_that_ended_is_refused() {
    let mut served = Served::new();
    let fake = FakeChannel::new(MAX_PENDING_LINES);
    let source = HtmlSource::File {
        path: served.path("index.html"),
        grant_root: Some(text(&served.docs)),
    };
    let stream = PageServeStream::start(Uuid::new_v4(), source, fake.clone()).unwrap();
    let work = stream.take(br#"{"id":3,"kind":"bridge","request":"{\"cmd\":\"tree\"}"}"#);
    assert!(served.origin.fixture().core.page_serve_work(work.unwrap()).is_none());
    assert_eq!(
        fake.frame(),
        Some(PageServeFrame::Failure {
            id: 3,
            error: "page closed".to_owned()
        })
    );
    stream.shutdown();
}

/// Upstream's `testABridgeRequestAdmittedBeforeItsPageClosedDoesNotRun`.
#[test]
fn a_bridge_request_admitted_before_its_page_closed_does_not_run() {
    let mut served = Served::new();
    let page = served.open_file(true);
    let channel = served.channel(&page);
    let work = channel
        .stream
        .take(br#"{"id":4,"kind":"bridge","request":"{\"cmd\":\"session.rename\",\"args\":{\"name\":\"renamed\"}}"}"#)
        .unwrap();
    assert!(served.origin.overlay(SessionCmd::OverlayClose, |_| {}).ok);
    assert!(served.origin.fixture().core.page_serve_work(work).is_none());
    assert!(channel.fake.closed());
    let session = served.origin.session();
    let name = served
        .origin
        .fixture()
        .store()
        .session(session)
        .unwrap()
        .custom_name
        .clone();
    assert_eq!(name, None);
}

// MARK: - how a channel ends

/// Upstream's `testAMalformedFrameEndsThePageAndKeepsThePresenter`, and a frame only the origin sends.
#[test]
fn a_malformed_or_origin_only_frame_ends_the_page_and_keeps_the_presenter() {
    for line in [&b"not a frame"[..], br#"{"id":1,"kind":"end"}"#] {
        let mut served = Served::new();
        let page = served.open_file(true);
        let channel = served.channel(&page);
        served.take(&channel, line);
        assert!(served.origin.slot_jobs().is_empty());
        assert_eq!(served.origin.outcome(&page)["outcome"], "dismissed");
        assert_eq!(served.page_closes().len(), 1);
        assert!(served.presenter_kept());
        assert_eq!(channel.fake.frame(), None);
        served.close(&channel);
        assert_eq!(served.page_closes().len(), 1, "ended once");
    }
}

/// Upstream's `testLosingTheHelperEndsOnlyThePage` and `testAClaimWhoseReplyFailsEndsThePage` (an abandoned connection
/// reports its close the same way).
#[test]
fn losing_the_helper_ends_only_the_page() {
    let mut served = Served::new();
    let page = served.open_file(true);
    let channel = served.channel(&page);
    served.close(&channel);
    assert!(served.origin.slot_jobs().is_empty());
    assert_eq!(served.page_closes().len(), 1);
    assert_eq!(served.origin.outcome(&page)["outcome"], "dismissed");
    assert!(served.presenter_kept());
    assert!(!served.origin.fixture().core.page_serve_adopted(page_id(&page)));
}

/// Upstream's `testLosingThePresenterEndsThePageAndClosesItsChannel`.
#[test]
fn losing_the_presenter_ends_the_page_and_closes_its_channel() {
    let mut served = Served::new();
    let page = served.open_file(true);
    let channel = served.channel(&page);
    let serial = served.origin.serial;
    let core = &mut served.origin.fixture().core;
    core.presentation_closed(serial);
    core.commit();
    assert!(served.origin.slot_jobs().is_empty());
    assert_eq!(served.origin.outcome(&page)["outcome"], "dismissed");
    assert!(channel.fake.closed());
    served.close(&channel);
    assert!(served.origin.slot_jobs().is_empty());
}

#[test]
fn a_page_closed_here_shuts_its_channel_once() {
    let mut served = Served::new();
    let page = served.open_file(true);
    let channel = served.channel(&page);
    let session = wire(served.origin.session());
    let closed = served
        .origin
        .fixture()
        .session(SessionCmd::OverlayClose, Some(&session), args(|_| {}));
    assert!(closed.ok);
    served.origin.fixture().core.commit();
    served.close(&channel);
    assert_eq!(channel.fake.shutdowns(), 1);
    assert_eq!(served.page_closes().len(), 1);
}
