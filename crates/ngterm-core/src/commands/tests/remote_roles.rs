//! Both roles of a handed-over page in one process (upstream's `ControlServerRemotePresentationTests` page cases:
//! `testAFilePageHandedToTheViewerSubmitsThroughItsChannel`, `testTheOriginReadsTheViewersPageStateAndPresenterLoss
//! DismissesIt`, `testAnOlderPresenterKeepsTheRefusal`): an origin core presenting a live session and a viewer core
//! attached to it, their presentation stream and the page's channel carried by in-memory queues instead of ssh. The
//! origin serves a real file from a temp folder off its main loop, as it does in the app.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ngterm_proto::command::{LiveCmd, SessionCmd};
use ngterm_proto::{Command, Request};
use serde_json::{Value, json};
use uuid::Uuid;

use super::live::{Live, live};
use super::remote::{node, set_lead};
use super::viewer::{Viewer, viewer_built};
use super::{Fixture, args, wire};
use crate::html::{HtmlHost, HtmlLoadState, HtmlNavigation, HtmlPageInfo};
use crate::remote::LineChannel;
use crate::remote::page_channel::{ChannelLink, Permits};
use crate::remote::page_serve::{PageChannel, PageServeStream};
use crate::remote::replica_page::{HelperKey, PageHelperLauncher, PageResource};

/// How long the pump waits for the origin's file thread before it calls the roles quiet.
const SETTLE: Duration = Duration::from_millis(300);
const PATIENCE: Duration = Duration::from_secs(10);

struct Host;

impl HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }

    fn navigate(&self, _: Uuid, _: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
}

/// The origin's end of the presentation stream: what it sends waits here for the viewer.
#[derive(Clone, Default)]
struct StreamQueue(Rc<RefCell<VecDeque<Vec<u8>>>>);

impl LineChannel for StreamQueue {
    fn send(&self, line: Vec<u8>) -> bool {
        self.0.borrow_mut().push_back(line);
        true
    }

    fn shutdown(&self) {}
}

/// The origin's end of a page channel, written from its file and reply threads; a line comes without its newline, which
/// the connection adds.
#[derive(Default)]
struct ChannelQueue {
    lines: Mutex<VecDeque<Vec<u8>>>,
    shut: Mutex<bool>,
}

impl ChannelQueue {
    /// The next line the viewer's reader takes, while it gets a permit.
    fn next_line(&self, permits: &Permits) -> Option<Vec<u8>> {
        let mut lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        if lines.is_empty() || !permits.try_take() {
            return None;
        }
        lines.pop_front()
    }
}

impl PageChannel for ChannelQueue {
    fn send(&self, line: Vec<u8>) -> bool {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(line);
        true
    }

    fn send_waiting(&self, line: Vec<u8>) -> bool {
        self.send(line)
    }

    fn shutdown(&self) {
        *self.shut.lock().unwrap_or_else(PoisonError::into_inner) = true;
    }
}

/// The viewer's helper: what its channel writes waits here for the origin.
#[derive(Clone, Default)]
struct HelperQueue(Rc<RefCell<HelperState>>);

#[derive(Default)]
struct HelperState {
    lines: VecDeque<Vec<u8>>,
    stopped: bool,
}

impl ChannelLink for HelperQueue {
    fn send(&self, line: Vec<u8>) -> bool {
        self.0.borrow_mut().lines.push_back(line);
        true
    }

    fn stop(&self) {
        self.0.borrow_mut().stopped = true;
    }
}

/// One helper the viewer started, and once claimed, the origin's stream it talks to.
struct Helper {
    argv: Vec<String>,
    key: HelperKey,
    permits: Arc<Permits>,
    queue: HelperQueue,
    served: Option<(Arc<PageServeStream>, Arc<ChannelQueue>)>,
}

#[derive(Default)]
struct Launcher(RefCell<Vec<Helper>>);

impl PageHelperLauncher for Launcher {
    fn launch(&self, argv: &[String], key: HelperKey, permits: Arc<Permits>) -> Result<Box<dyn ChannelLink>, String> {
        let queue = HelperQueue::default();
        self.0.borrow_mut().push(Helper {
            argv: argv.to_vec(),
            key,
            permits,
            queue: queue.clone(),
            served: None,
        });
        Ok(Box::new(queue))
    }
}

/// An origin presenting a live session to a viewer core.
struct Roles {
    origin: Live,
    serial: u64,
    to_viewer: StreamQueue,
    viewer: Viewer,
    launcher: Rc<Launcher>,
    /// The viewer's frames already handed to the origin.
    forwarded: usize,
    _dir: tempfile::TempDir,
    docs: PathBuf,
}

impl Roles {
    /// A viewer that shows pages when `pages`.
    fn new(pages: bool) -> Self {
        let mut origin = live();
        origin.fixture.core.set_html_host(Box::new(Host));
        let left = origin.left;
        set_lead(&mut origin.fixture.core, left, "follower");
        let session = origin.session;
        let response = origin.fixture.core.dispatch(&Request {
            cmd: Command::Live(LiveCmd::Present),
            target: Some(wire(session)),
            args: None,
        });
        assert!(response.ok, "{response:?}");
        let to_viewer = StreamQueue::default();
        let serial = origin
            .fixture
            .core
            .adopt_presentation_stream(&wire(session), Box::new(to_viewer.clone()));
        let launcher = Rc::new(Launcher::default());
        let helpers = Rc::clone(&launcher);
        let viewer = viewer_built(false, move |fixture: &mut Fixture| {
            if pages {
                fixture.core.set_html_host(Box::new(Host));
                fixture.core.set_page_helper_launcher(helpers);
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let docs = std::fs::canonicalize(dir.path()).unwrap().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(
            docs.join("index.html"),
            "<link rel=stylesheet href=style.css><p id=p>x</p>",
        )
        .unwrap();
        std::fs::write(docs.join("style.css"), "#p { color: rgb(1, 2, 3); }").unwrap();
        let mut roles = Self {
            origin,
            serial,
            to_viewer,
            viewer,
            launcher,
            forwarded: 0,
            _dir: dir,
            docs,
        };
        roles.pump();
        roles
    }

    fn path(&self, name: &str) -> String {
        self.docs.join(name).to_string_lossy().into_owned()
    }

    /// Moves what either side sent to the other until both are quiet: the stream both ways, the helpers' claims, the
    /// page channels both ways (the origin's file thread given time to answer).
    fn pump(&mut self) {
        let mut quiet_since = None;
        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline {
            if self.step() {
                quiet_since = None;
            } else {
                let since = *quiet_since.get_or_insert_with(Instant::now);
                if since.elapsed() > SETTLE {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    /// One round of moving lines; whether anything moved.
    fn step(&mut self) -> bool {
        let mut moved = false;
        let sent: Vec<Value> = self.viewer.link().0.borrow().sent[self.forwarded..].to_vec();
        self.forwarded += sent.len();
        for frame in sent {
            let line = serde_json::to_vec(&frame).unwrap();
            self.origin.fixture.core.presentation_line(self.serial, &line);
            self.origin.fixture.core.commit();
            moved = true;
        }
        let lines: Vec<Vec<u8>> = self.to_viewer.0.borrow_mut().drain(..).collect();
        let key = self.viewer.link().0.borrow().key.unwrap();
        for line in lines {
            self.viewer.fixture.core.viewer_line(key, &line);
            self.viewer.fixture.core.commit();
            moved = true;
        }
        moved |= self.claim_helpers();
        moved |= self.carry_page_channels();
        moved
    }

    /// Claims each new helper's page on the origin, as `ngtermctl session overlay page-serve TOKEN` does over ssh.
    fn claim_helpers(&mut self) -> bool {
        let unclaimed: Vec<(usize, String)> = self
            .launcher
            .0
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, helper)| helper.served.is_none() && !helper.queue.0.borrow().stopped)
            .map(|(index, helper)| (index, token_of(&helper.argv)))
            .collect();
        let mut moved = false;
        for (index, token) in unclaimed {
            let response = self.origin.fixture.core.dispatch(&Request {
                target: Some(token),
                ..Request::new(Command::Session(SessionCmd::OverlayPageServe))
            });
            assert!(response.ok, "the claim: {response:?}");
            let page = response.result.unwrap()["id"].as_str().unwrap().to_owned();
            let channel = Arc::new(ChannelQueue::default());
            let stream = self
                .origin
                .fixture
                .core
                .adopt_page_serve_stream(&page, Arc::clone(&channel) as Arc<dyn PageChannel>)
                .expect("the channel is adopted");
            self.launcher.0.borrow_mut()[index].served = Some((stream, channel));
            moved = true;
        }
        moved
    }

    /// Carries each claimed helper's lines to the origin's stream and the origin's answers back to the viewer, a line
    /// back only while the viewer's reader gets a permit.
    fn carry_page_channels(&mut self) -> bool {
        let mut moved = false;
        let count = self.launcher.0.borrow().len();
        for index in 0..count {
            let (key, permits, queue, served) = {
                let helpers = self.launcher.0.borrow();
                let helper = &helpers[index];
                let Some((stream, channel)) = helper.served.as_ref() else {
                    continue;
                };
                (
                    helper.key,
                    Arc::clone(&helper.permits),
                    helper.queue.clone(),
                    (Arc::clone(stream), Arc::clone(channel)),
                )
            };
            let (stream, channel) = served;
            let outgoing: Vec<Vec<u8>> = queue.0.borrow_mut().lines.drain(..).collect();
            for mut line in outgoing {
                // the helper writes the channel's lines as they come, newline included
                line.pop();
                moved = true;
                if let Some(work) = stream.take(&line)
                    && let Some(call) = self.origin.fixture.core.page_serve_work(work)
                {
                    let response = self.origin.fixture.core.dispatch(&call.request);
                    call.reply.answer(&response);
                }
                self.origin.fixture.core.commit();
            }
            while let Some(line) = channel.next_line(&permits) {
                self.viewer.fixture.core.page_helper_line(key, &line);
                self.viewer.fixture.core.commit();
                moved = true;
            }
        }
        moved
    }

    /// Opens the folder's index on the origin as a `--js --cwd` file page; the response.
    fn open_page(&mut self) -> ngterm_proto::Response {
        let (index, docs) = (self.path("index.html"), self.docs.to_string_lossy().into_owned());
        let session = wire(self.origin.session);
        let response = self.origin.fixture.session(
            SessionCmd::OverlayOpen,
            Some(&session),
            args(|a| {
                a.html = Some(index);
                a.cwd = Some(docs);
                a.javascript = Some(true);
            }),
        );
        self.origin.fixture.core.commit();
        self.pump();
        response
    }

    /// The viewer's replica of the session-wide page.
    fn replica(&self) -> Option<Uuid> {
        self.viewer.session().overlay.page().map(|page| page.id)
    }

    fn origin_node(&mut self) -> Value {
        let session = self.origin.session;
        node(&mut self.origin.fixture, session)
    }

    fn outcome(&mut self, page: &str) -> Value {
        let response = self.origin.fixture.session(
            SessionCmd::OverlayResult,
            None,
            args(|a| a.page = Some(page.to_owned())),
        );
        response.result.unwrap_or(Value::Null)["pageOutcome"].clone()
    }
}

/// The token the helper's remote command claims: the UUID after `page-serve`, however the shell chain quotes it.
fn token_of(argv: &[String]) -> String {
    let remote = argv.last().unwrap();
    let (_, tail) = remote.split_once("page-serve ").unwrap();
    tail.trim_start_matches(|c: char| !c.is_ascii_hexdigit())
        .chars()
        .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
        .collect()
}

/// Upstream's `testAFilePageHandedToTheViewerSubmitsThroughItsChannel`.
#[test]
#[cfg_attr(
    windows,
    ignore = "a file page's path is a `/` one (`html::grant_error`) and this page a real file in the temporary directory; Windows' file pages come with the HTML overlays' Windows plan"
)]
fn a_file_page_handed_to_the_viewer_loads_and_submits_through_its_channel() {
    let mut roles = Roles::new(true);
    assert_eq!(roles.viewer.node()["presentation"]["mode"], json!("presenter"));

    let opened = roles.open_page();

    assert!(opened.ok, "{opened:?}");
    let page = opened.result.unwrap()["pageID"].as_str().unwrap().to_owned();
    let local = roles.replica().expect("the viewer shows the page");
    assert_eq!(roles.launcher.0.borrow().len(), 1);
    assert!(roles.viewer.fixture.core.linked_replica(local));

    // the web view loads the page's stylesheet through the channel
    let style: Rc<RefCell<Option<Result<PageResource, String>>>> = Rc::default();
    let into = Rc::clone(&style);
    let path = roles.path("style.css");
    roles
        .viewer
        .fixture
        .core
        .replica_fetch(local, &path, Box::new(move |result| *into.borrow_mut() = Some(result)));
    roles.pump();
    let fetched = style.borrow_mut().take().expect("the stylesheet arrived");
    assert_eq!(
        fetched,
        Ok(PageResource {
            mime_type: "text/css".to_owned(),
            body: b"#p { color: rgb(1, 2, 3); }".to_vec(),
        })
    );

    // a file outside the page's folder stays on the origin
    let outside: Rc<RefCell<Option<Result<PageResource, String>>>> = Rc::default();
    let into = Rc::clone(&outside);
    roles.viewer.fixture.core.replica_fetch(
        local,
        "/etc/passwd",
        Box::new(move |result| *into.borrow_mut() = Some(result)),
    );
    roles.pump();
    assert_eq!(
        outside.borrow_mut().take(),
        Some(Err("file is outside the page's folder".to_owned()))
    );

    // the page's own submit goes through its channel into the origin's ledger
    let reply: Rc<RefCell<Option<Result<Value, String>>>> = Rc::default();
    let into = Rc::clone(&reply);
    roles.viewer.fixture.core.replica_bridge(
        local,
        r#"{"cmd":"session.overlay.submit","args":{"value":"done"}}"#,
        Box::new(move |result| *into.borrow_mut() = Some(result)),
    );
    roles.pump();

    assert_eq!(
        roles.outcome(&page),
        json!({"pageID": page, "outcome": "submitted", "value": "done"})
    );
    assert!(
        roles.replica().is_none(),
        "the origin closed the page, and the viewer took its replica down"
    );
    assert!(
        roles.launcher.0.borrow()[0].queue.0.borrow().stopped,
        "the helper stopped with the page"
    );
    assert!(roles.origin_node().get("htmlOverlays").is_none());
}

/// Upstream's `testTheOriginReadsTheViewersPageStateAndPresenterLossDismissesIt`.
#[test]
#[cfg_attr(
    windows,
    ignore = "a file page's path is a `/` one (`html::grant_error`) and this page a real file in the temporary directory; Windows' file pages come with the HTML overlays' Windows plan"
)]
fn the_origin_reads_the_viewers_page_state_and_presenter_loss_dismisses_it() {
    let mut roles = Roles::new(true);
    let opened = roles.open_page();
    let page = opened.result.unwrap()["pageID"].as_str().unwrap().to_owned();
    let local = roles.replica().unwrap();

    let core = &mut roles.viewer.fixture.core;
    core.set_html_load_state(local, HtmlLoadState::Loaded, None);
    let index = roles.path("index.html");
    roles.viewer.fixture.core.set_html_page(
        local,
        Some(HtmlPageInfo {
            page: index.clone(),
            title: Some("Docs".to_owned()),
            can_go_back: false,
            can_go_forward: false,
        }),
    );
    roles.viewer.fixture.core.commit();
    roles.pump();

    let node = roles.origin_node();
    let read = &node["htmlOverlays"][0];
    assert_eq!(read["remote"], json!(true));
    assert_eq!(read["state"], json!("loaded"));
    assert_eq!(read["title"], json!("Docs"));
    assert_eq!(read["page"], json!(index));
    assert_eq!(roles.outcome(&page)["outcome"], "pending");

    // the presenter goes: the origin dismisses the page, the viewer tears down the replica and its helper
    let serial = roles.serial;
    roles.origin.fixture.core.presentation_closed(serial);
    roles.origin.fixture.core.commit();
    let key = roles.viewer.link().0.borrow().key.unwrap();
    roles.viewer.fixture.core.viewer_closed(key, "exit 255");
    roles.viewer.fixture.core.commit();
    roles.pump();

    assert_eq!(roles.outcome(&page)["outcome"], "dismissed");
    assert!(roles.origin_node().get("htmlOverlays").is_none());
    assert!(roles.replica().is_none());
    assert!(roles.launcher.0.borrow()[0].queue.0.borrow().stopped);
}

/// Upstream's `testAnOlderPresenterKeepsTheRefusal`: a viewer that shows no pages lists no page kind, so the origin
/// keeps its refusal rather than handing the page over.
#[test]
#[cfg_attr(
    windows,
    ignore = "a file page's path is a `/` one (`html::grant_error`) and this page a real file in the temporary directory; Windows' file pages come with the HTML overlays' Windows plan"
)]
fn a_viewer_showing_no_pages_keeps_the_origins_refusal() {
    let mut roles = Roles::new(false);

    let opened = roles.open_page();

    assert_eq!(
        opened.error.as_deref(),
        Some("a viewer presents this session: an html overlay would open where nobody sees it")
    );
    assert!(roles.replica().is_none());
    assert!(roles.launcher.0.borrow().is_empty());
}
