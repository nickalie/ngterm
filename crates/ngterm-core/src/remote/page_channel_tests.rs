//! Upstream's `RemotePageChannelTests`, over a fake helper and a fake origin driven line by line: the origin's lines
//! reach the channel only while the reader gets a permit, as the helper's reader thread does.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use super::*;

const INDEX: &[u8] = b"<p>index</p>";
/// The fake origin's chunk size: small, so a 300 KiB file outlasts the reader's permits.
const CHUNK: usize = 1024;

fn big() -> Vec<u8> {
    (0..300 * 1024)
        .map(|byte: usize| u8::try_from(byte % 251).unwrap())
        .collect()
}

#[derive(Default)]
struct LinkState {
    received: Vec<PageServeFrame>,
    refusing: bool,
    stopped: bool,
}

#[derive(Clone, Default)]
struct Link(Rc<RefCell<LinkState>>);

impl ChannelLink for Link {
    fn send(&self, line: Vec<u8>) -> bool {
        let mut state = self.0.borrow_mut();
        if state.refusing {
            return false;
        }
        state
            .received
            .push(page_serve::decode(&line[..line.len() - 1]).unwrap());
        true
    }

    fn stop(&self) {
        self.0.borrow_mut().stopped = true;
    }
}

/// The origin's side: answers each request it got as `ngtermctl session overlay page-serve` would, into a queue of
/// lines the reader hands over one permit at a time.
struct Origin {
    link: Link,
    files: HashMap<&'static str, (&'static str, Vec<u8>)>,
    /// Paths it holds without answering until a cancel arrives (and only then, with `hold_cancels`, when told).
    held: Vec<&'static str>,
    hold_cancels: bool,
    reply: Option<&'static str>,
    seen: usize,
    lines: VecDeque<Vec<u8>>,
    held_cancels: Vec<RequestId>,
}

impl Origin {
    fn new(link: Link) -> Self {
        let mut files = HashMap::new();
        files.insert("/docs/index.html", ("text/html", INDEX.to_vec()));
        files.insert("/docs/big.bin", ("application/octet-stream", big()));
        Self {
            link,
            files,
            held: Vec::new(),
            hold_cancels: false,
            reply: Some(r#"{"ok":true,"result":{}}"#),
            seen: 0,
            lines: VecDeque::new(),
            held_cancels: Vec::new(),
        }
    }

    fn line(&mut self, frame: &PageServeFrame) {
        let mut line = page_serve::encode(frame).unwrap();
        line.pop();
        self.lines.push_back(line);
    }

    /// Answers what arrived since the last call.
    fn answer(&mut self) {
        let received: Vec<PageServeFrame> = self.link.0.borrow().received[self.seen..].to_vec();
        self.seen += received.len();
        for frame in received {
            match frame {
                PageServeFrame::Resource { id, path } if self.held.contains(&path.as_str()) => {
                    let _ = id;
                }
                PageServeFrame::Resource { id, path } => match self.files.get(path.as_str()).cloned() {
                    Some((mime_type, data)) => {
                        self.line(&PageServeFrame::Head {
                            id,
                            mime_type: mime_type.to_owned(),
                        });
                        for chunk in data.chunks(CHUNK) {
                            self.line(&PageServeFrame::Chunk {
                                id,
                                data: chunk.to_vec(),
                            });
                        }
                        self.line(&PageServeFrame::End { id });
                    }
                    None => self.line(&PageServeFrame::Failure {
                        id,
                        error: "no such file".to_owned(),
                    }),
                },
                PageServeFrame::Cancel { id } if self.hold_cancels => self.held_cancels.push(id),
                PageServeFrame::Cancel { id } => self.line(&PageServeFrame::Failure {
                    id,
                    error: "cancelled".to_owned(),
                }),
                PageServeFrame::Bridge { id, .. } => {
                    if let Some(reply) = self.reply {
                        self.line(&PageServeFrame::Reply {
                            id,
                            response: reply.to_owned(),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn answer_cancels(&mut self) {
        for id in std::mem::take(&mut self.held_cancels) {
            self.line(&PageServeFrame::Failure {
                id,
                error: "cancelled".to_owned(),
            });
        }
    }

    fn resources(&self) -> Vec<RequestId> {
        self.link
            .0
            .borrow()
            .received
            .iter()
            .filter_map(|frame| match frame {
                PageServeFrame::Resource { id, .. } => Some(*id),
                _ => None,
            })
            .collect()
    }

    fn cancels(&self) -> Vec<RequestId> {
        self.link
            .0
            .borrow()
            .received
            .iter()
            .filter_map(|frame| match frame {
                PageServeFrame::Cancel { id } => Some(*id),
                _ => None,
            })
            .collect()
    }
}

/// A channel, its origin and everything the channel told its owner.
struct Harness {
    channel: ReplicaChannel,
    permits: Arc<Permits>,
    origin: Origin,
    events: Vec<ChannelEvent>,
    /// The lines the reader handed over.
    delivered: usize,
}

impl Harness {
    fn new() -> Self {
        let link = Link::default();
        let permits = ReplicaChannel::permits();
        Self {
            channel: ReplicaChannel::new(Box::new(link.clone()), Arc::clone(&permits)),
            permits,
            origin: Origin::new(link),
            events: Vec::new(),
            delivered: 0,
        }
    }

    fn fetch(&mut self, path: &str) -> RequestId {
        self.fetch_at(path, 0)
    }

    fn fetch_at(&mut self, path: &str, revision: u64) -> RequestId {
        let (id, events) = self.channel.fetch(path, revision).unwrap();
        self.events.extend(events);
        id
    }

    fn bridge(&mut self, request: &str) -> Result<RequestId, ChannelFailure> {
        let (id, events) = self.channel.bridge(request)?;
        self.events.extend(events);
        Ok(id)
    }

    /// The reader hands over the origin's lines while it gets a permit; the origin answers what it got meanwhile.
    fn pump(&mut self) {
        loop {
            self.origin.answer();
            if self.origin.lines.is_empty() || !self.permits.try_take() {
                return;
            }
            let line = self.origin.lines.pop_front().unwrap();
            self.delivered += 1;
            let events = self.channel.receive(&line);
            self.events.extend(events);
        }
    }

    /// Pulls resource `id`'s body whole, pumping as the reader would; its end.
    fn read(&mut self, id: RequestId) -> Result<Vec<u8>, ChannelFailure> {
        let mut body = Vec::new();
        for _ in 0..100_000 {
            self.pump();
            if let Some(result) = self.finished(id) {
                return result.map(|()| body);
            }
            match self.channel.pull(id) {
                Pull::Chunk(chunk) => body.extend(chunk),
                Pull::Done(result) => return result.map(|()| body),
                Pull::Wait => {}
            }
        }
        panic!("resource {id} never ended");
    }

    fn finished(&mut self, id: RequestId) -> Option<Result<(), ChannelFailure>> {
        let index = self
            .events
            .iter()
            .position(|event| matches!(event, ChannelEvent::Finished { id: done, .. } if *done == id))?;
        match self.events.remove(index) {
            ChannelEvent::Finished { result, .. } => Some(result),
            _ => None,
        }
    }

    fn head(&self, id: RequestId) -> Option<String> {
        self.events.iter().find_map(|event| match event {
            ChannelEvent::Head { id: head, mime_type } if *head == id => Some(mime_type.clone()),
            _ => None,
        })
    }

    fn reply(&mut self, id: RequestId) -> Option<Result<String, ChannelFailure>> {
        self.pump();
        self.events.iter().find_map(|event| match event {
            ChannelEvent::Reply { id: reply, result } if *reply == id => Some(result.clone()),
            _ => None,
        })
    }

    fn closes(&self) -> usize {
        self.events
            .iter()
            .filter(|event| matches!(event, ChannelEvent::Closed { .. }))
            .count()
    }
}

#[test]
fn a_fetch_streams_the_file_in_order() {
    let mut harness = Harness::new();

    let id = harness.fetch("/docs/big.bin");
    let data = harness.read(id).unwrap();

    assert_eq!(harness.head(id).as_deref(), Some("application/octet-stream"));
    assert_eq!(data, big());
    assert_eq!(harness.origin.resources(), [1]);
    assert_eq!(harness.permits.free(), INBOUND_LIMIT, "every permit came back");
}

#[test]
fn a_stalled_consumer_stops_the_reader_until_it_pulls() {
    let mut harness = Harness::new();
    let id = harness.fetch("/docs/big.bin");

    harness.pump();
    let stalled = harness.delivered;
    assert!(
        stalled < 300,
        "the reader must stop taking chunks the page has not consumed: {stalled}"
    );
    assert_eq!(harness.permits.free(), 0);
    harness.pump();
    assert_eq!(harness.delivered, stalled);

    let data = harness.read(id).unwrap();
    assert_eq!(data.len(), 300 * 1024);
}

#[test]
fn cancelling_a_stalled_transfer_frees_its_buffered_chunks() {
    let mut harness = Harness::new();
    let id = harness.fetch("/docs/big.bin");
    harness.pump();
    assert!(harness.delivered < 300);

    assert!(harness.channel.cancel(id).is_empty());
    assert!(
        harness.permits.free() > 0,
        "the buffered chunks gave their permits back"
    );

    let next = harness.fetch("/docs/index.html");
    assert_eq!(harness.read(next).unwrap(), INDEX);
    assert_eq!(harness.origin.cancels(), [1]);
    assert_eq!(
        harness.finished(id),
        None,
        "the owner withdrew it and hears nothing more of it"
    );
}

#[test]
fn a_cancel_after_the_whole_answer_arrived_sends_nothing() {
    let mut harness = Harness::new();
    let id = harness.fetch("/docs/index.html");
    harness.pump();
    // the head and the chunk arrived, the end too: nothing is in flight on the origin
    let id_pulled = harness.channel.pull(id);
    assert_eq!(id_pulled, Pull::Chunk(INDEX.to_vec()));

    harness.channel.cancel(id);

    assert!(harness.origin.cancels().is_empty());
    let next = harness.fetch("/docs/index.html");
    assert_eq!(harness.read(next).unwrap(), INDEX);
}

#[test]
fn cancel_before_head_sends_cancel_and_keeps_the_slot_until_the_origin_ends_it() {
    let mut harness = Harness::new();
    harness.origin.held = vec!["/docs/held.bin"];
    harness.origin.hold_cancels = true;
    let waiting: Vec<RequestId> = (0..RESOURCE_LIMIT).map(|_| harness.fetch("/docs/held.bin")).collect();
    harness.pump();
    assert_eq!(harness.origin.resources().len(), RESOURCE_LIMIT);

    harness.channel.cancel(waiting[0]);
    harness.pump();
    assert_eq!(harness.origin.cancels(), [1]);

    let next = harness.fetch("/docs/index.html");
    harness.pump();
    assert_eq!(
        harness.origin.resources().len(),
        RESOURCE_LIMIT,
        "the cancelled request keeps its slot until the origin ends it"
    );

    harness.origin.answer_cancels();
    assert_eq!(harness.read(next).unwrap(), INDEX);

    for _ in 0..2 {
        harness.fetch("/docs/held.bin");
    }
    harness.pump();
    assert_eq!(
        harness.origin.resources().len(),
        RESOURCE_LIMIT + 2,
        "the slot was freed once, not twice"
    );
}

#[test]
fn cancel_while_queued_here_sends_nothing() {
    let mut harness = Harness::new();
    harness.origin.held = vec!["/docs/held.bin"];
    for _ in 0..RESOURCE_LIMIT {
        harness.fetch("/docs/held.bin");
    }
    harness.pump();
    let queued = harness.fetch("/docs/index.html");
    harness.pump();

    assert!(harness.channel.cancel(queued).is_empty());

    harness.pump();
    assert_eq!(harness.origin.resources().len(), RESOURCE_LIMIT);
    assert!(harness.origin.cancels().is_empty());
}

#[test]
fn closing_fails_pending_head_and_bridge_calls() {
    let mut harness = Harness::new();
    harness.origin.held = vec!["/docs/held.bin"];
    harness.origin.reply = None;
    let fetch = harness.fetch("/docs/held.bin");
    let bridge = harness.bridge(r#"{"cmd":"tree"}"#).unwrap();
    harness.pump();
    assert_eq!(harness.origin.link.0.borrow().received.len(), 2);

    let events = harness.channel.close("test");

    assert_eq!(
        events,
        [
            ChannelEvent::Finished {
                id: fetch,
                result: Err(ChannelFailure::Closed)
            },
            ChannelEvent::Reply {
                id: bridge,
                result: Err(ChannelFailure::Closed)
            },
            ChannelEvent::Closed {
                reason: "test".to_owned()
            },
        ]
    );
    assert_eq!(ChannelFailure::Closed.to_string(), "page channel closed");
    assert!(harness.origin.link.0.borrow().stopped);
}

#[test]
fn an_answer_for_a_load_the_page_left_is_dropped() {
    let mut harness = Harness::new();
    let id = harness.fetch_at("/docs/big.bin", 1);
    harness.pump();

    let events = harness.channel.revision_moved(2);

    assert_eq!(
        events,
        [ChannelEvent::Finished {
            id,
            result: Err(ChannelFailure::Cancelled)
        }]
    );
    assert_eq!(harness.origin.cancels(), [1]);
    assert_eq!(harness.channel.pull(id), Pull::Done(Err(ChannelFailure::Cancelled)));
    // the rest of the stale answer is drained and dropped, its permits given back
    harness.pump();
    harness.pump();
    assert_eq!(harness.permits.free(), INBOUND_LIMIT);
    let current = harness.fetch_at("/docs/index.html", 2);
    assert!(
        harness.channel.revision_moved(2).is_empty(),
        "a request of the load shown stays"
    );
    assert_eq!(harness.read(current).unwrap(), INDEX);
}

#[test]
fn a_bridge_reply_comes_back_as_the_origin_sent_it() {
    let mut harness = Harness::new();
    harness.origin.reply = Some(r#"{"ok":true,"result":{"extra":{"nested":"kept"}}}"#);

    let id = harness.bridge(r#"{"cmd":"tree"}"#).unwrap();

    assert_eq!(
        harness.reply(id),
        Some(Ok(r#"{"ok":true,"result":{"extra":{"nested":"kept"}}}"#.to_owned()))
    );
    assert_eq!(
        crate::html::bridge::shape_raw_reply(r#"{"ok":true,"result":{"extra":{"nested":"kept"}}}"#),
        Ok(serde_json::json!({"extra": {"nested": "kept"}}))
    );
}

#[test]
fn failures_before_head_keep_the_channel_usable() {
    let mut harness = Harness::new();

    for _ in 0..(RESOURCE_LIMIT * 2) {
        let id = harness.fetch("/docs/missing.html");
        assert_eq!(
            harness.read(id),
            Err(ChannelFailure::Refused("no such file".to_owned()))
        );
    }

    let next = harness.fetch("/docs/index.html");
    assert_eq!(harness.read(next).unwrap(), INDEX);
    assert!(harness.channel.transfers.is_empty(), "a failed request keeps nothing");
}

#[test]
fn an_answer_ending_before_its_head_fails() {
    let mut harness = Harness::new();
    harness.origin.held = vec!["/docs/held.bin"];
    let id = harness.fetch("/docs/held.bin");
    harness.origin.line(&PageServeFrame::End { id });

    assert_eq!(
        harness.read(id),
        Err(ChannelFailure::Refused("empty answer".to_owned()))
    );
}

#[test]
fn an_oversized_bridge_request_fails_alone_and_the_channel_stays() {
    let mut harness = Harness::new();
    harness.origin.reply = Some(r#"{"ok":true,"result":{"text":"ok"}}"#);
    let oversized = format!(
        r#"{{"cmd":"notify","args":{{"body":"{}"}}}}"#,
        "x".repeat(page_serve::MAX_FRAME_BYTES)
    );

    assert_eq!(harness.bridge(&oversized), Err(ChannelFailure::TooLarge));
    assert_eq!(
        ChannelFailure::TooLarge.to_string(),
        "request too large for the page channel"
    );

    let id = harness.bridge(r#"{"cmd":"tree"}"#).unwrap();
    assert_eq!(
        harness.reply(id),
        Some(Ok(r#"{"ok":true,"result":{"text":"ok"}}"#.to_owned()))
    );
    assert!(!harness.origin.link.0.borrow().stopped);
}

#[test]
fn bridge_calls_past_their_limit_wait_for_a_slot() {
    let mut harness = Harness::new();
    harness.origin.reply = None;
    let ids: Vec<RequestId> = (0..=BRIDGE_LIMIT)
        .map(|_| harness.bridge(r#"{"cmd":"tree"}"#).unwrap())
        .collect();
    harness.pump();
    let bridged = |harness: &Harness| {
        harness
            .origin
            .link
            .0
            .borrow()
            .received
            .iter()
            .filter(|frame| matches!(frame, PageServeFrame::Bridge { .. }))
            .count()
    };
    assert_eq!(bridged(&harness), BRIDGE_LIMIT);

    harness.origin.line(&PageServeFrame::Reply {
        id: ids[0],
        response: "{}".to_owned(),
    });
    harness.pump();

    assert_eq!(
        bridged(&harness),
        BRIDGE_LIMIT + 1,
        "the freed slot went to the waiting call"
    );
}

#[test]
fn helper_exit_closes_the_channel_once() {
    let mut harness = Harness::new();

    let first = harness.channel.close("helper exited");
    let second = harness.channel.close("helper exited");

    assert_eq!(
        first,
        [ChannelEvent::Closed {
            reason: "helper exited".to_owned()
        }]
    );
    assert!(second.is_empty());
    assert!(harness.origin.link.0.borrow().stopped);
    assert_eq!(
        harness.channel.fetch("/docs/index.html", 0),
        Err(ChannelFailure::Closed)
    );
}

#[test]
fn a_malformed_frame_or_one_only_this_machine_sends_closes_the_channel() {
    for line in [
        &b"not json"[..],
        br#"{"id":1,"kind":"cancel"}"#,
        br#"{"id":1,"kind":"resource","path":"/a"}"#,
    ] {
        let mut harness = Harness::new();
        assert!(harness.permits.try_take());
        harness.events = harness.channel.receive(line);
        assert_eq!(harness.closes(), 1, "{}", String::from_utf8_lossy(line));
        assert!(harness.origin.link.0.borrow().stopped);
    }
}

#[test]
fn a_helper_that_stops_taking_requests_closes_the_channel() {
    let mut harness = Harness::new();
    harness.origin.link.0.borrow_mut().refusing = true;

    let (id, events) = harness.channel.fetch("/docs/index.html", 0).unwrap();

    assert_eq!(
        events,
        [
            ChannelEvent::Finished {
                id,
                result: Err(ChannelFailure::Closed)
            },
            ChannelEvent::Closed {
                reason: "the channel stopped taking requests".to_owned()
            },
        ]
    );
}

#[test]
fn the_reader_stops_once_the_permits_close() {
    let permits = Permits::new(1);
    assert!(permits.take());
    let waiting = Arc::new(permits);
    let reader = Arc::clone(&waiting);
    let blocked = std::thread::spawn(move || reader.take());
    std::thread::sleep(std::time::Duration::from_millis(50));
    waiting.close();
    assert!(!blocked.join().unwrap(), "a closed channel delivers nothing more");
}
