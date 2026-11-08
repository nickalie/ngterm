//! Helper races and cleanup of pages that cannot be placed after their forward starts.

use std::cell::{Cell, RefCell};

use ngterm_proto::AppIdentity;
use serde_json::json;

use super::*;
use crate::model::{NewSession, RemoteBinding, WindowLibrary};
use crate::remote::{LinkKey, PresentationLink, PresentationTransport};
use crate::{CoreConfig, CoreState};

struct Host;

impl crate::html::HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }

    fn navigate(&self, _page: Uuid, _navigation: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone, Default)]
struct Helper(Rc<Cell<bool>>);

impl ChannelLink for Helper {
    fn send(&self, _line: Vec<u8>) -> bool {
        true
    }
    fn stop(&self) {
        self.0.set(true);
    }
}

#[derive(Default)]
struct Launcher {
    launches: RefCell<Vec<(HelperKey, Arc<Permits>, Helper)>>,
}

impl PageHelperLauncher for Launcher {
    fn launch(&self, _argv: &[String], key: HelperKey, permits: Arc<Permits>) -> Result<Box<dyn ChannelLink>, String> {
        let helper = Helper::default();
        self.launches.borrow_mut().push((key, permits, helper.clone()));
        Ok(Box::new(helper))
    }
}

fn core() -> (Core, Uuid, Rc<Launcher>) {
    let mut library = WindowLibrary::new("/tmp");
    let window = library.new_window(None);
    let store = library.store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let row = store
        .add_session(
            workspace,
            NewSession {
                remote_host: Some("studio".to_owned()),
                ..NewSession::at_cwd("/tmp")
            },
        )
        .unwrap();
    store.bind_remote(
        RemoteBinding::new(uuid_string(Uuid::new_v4()), HashMap::new(), None, None),
        row,
    );
    let mut core = Core::new(
        CoreConfig {
            app: AppIdentity::from_recorded("0.1.0", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            settings: crate::persist::AppSettings::default(),
            persistence: None,
        },
    );
    let launcher = Rc::new(Launcher::default());
    core.set_html_host(Box::new(Host));
    core.set_page_helper_launcher(launcher.clone());
    (core, row, launcher)
}

/// The origin's end of a row's presentation stream: every frame this machine sent it.
#[derive(Clone, Default)]
struct Origin(Rc<RefCell<Vec<serde_json::Value>>>);

impl PresentationLink for Origin {
    fn send(&self, line: &[u8]) {
        self.0.borrow_mut().push(serde_json::from_slice(line).unwrap());
    }

    fn stop(&self) {}
}

impl PresentationTransport for Origin {
    fn open(&self, _argv: &[String], _key: LinkKey) -> Box<dyn PresentationLink> {
        Box::new(self.clone())
    }
}

impl Origin {
    /// Opens `row`'s presentation stream onto a new origin that records what it hears.
    fn listening(core: &mut Core, row: Uuid) -> Self {
        let origin = Self::default();
        let store = core.library_mut().store_for_session_mut(row).unwrap();
        store.bind_remote(
            RemoteBinding::new(uuid_string(Uuid::new_v4()), HashMap::new(), Some(1), None),
            row,
        );
        core.set_presentation_transport(Rc::new(origin.clone()));
        core.start_remote_presentation(row);
        origin
    }

    /// The page payloads of the frames of `kind` the origin heard, in order.
    fn heard(&self, kind: &str) -> Vec<serde_json::Value> {
        self.0
            .borrow()
            .iter()
            .filter(|frame| frame["kind"] == kind)
            .map(|frame| frame["page"].clone())
            .collect()
    }
}

fn page(source: &serde_json::Value) -> PresentationPage {
    serde_json::from_value(json!({
        "page": uuid_string(Uuid::new_v4()), "token": "tok-1", "identity": "report.html", "source": source,
        "chromeless": false, "javascript": true, "navigation": false, "browse": false,
        "persistent": false, "follow": false
    }))
    .unwrap()
}

fn loopback() -> PresentationPage {
    page(&json!({"url": {"_0": "http://localhost:8000/report"}}))
}

#[test]
fn a_forward_stops_if_its_page_cannot_be_placed_when_it_becomes_ready() {
    let (mut core, row, launcher) = core();
    let origin = Origin::listening(&mut core, row);
    let mut page = loopback();
    page.page = "invalid".to_owned();
    core.show_replica_page(&page, row).unwrap();
    let (key, permits, helper) = launcher.launches.borrow()[0].clone();
    assert!(!helper.0.get());
    assert!(permits.take());
    assert!(origin.heard("page.rejected").is_empty());

    core.page_helper_line(key, READY_LINE.as_bytes());

    assert!(helper.0.get());
    assert!(!permits.take());
    assert!(core.library().held_session(row).unwrap().overlay.page().is_none());
    assert!(core.remote.viewer.pages.pending.is_empty());
    assert_eq!(
        origin.heard("page.rejected"),
        [json!({"page": "invalid", "reason": "invalid page id"})]
    );
    // An already queued report from the stopped helper cannot put the refused page back, nor refuse it twice.
    core.page_helper_line(key, READY_LINE.as_bytes());
    assert!(core.library().held_session(row).unwrap().overlay.page().is_none());
    assert_eq!(origin.heard("page.rejected").len(), 1);
}

#[test]
fn stale_forward_reports_leave_the_current_launch_running() {
    let (mut core, row, launcher) = core();
    core.show_replica_page(&loopback(), row).unwrap();
    let (key, permits, helper) = launcher.launches.borrow()[0].clone();
    let stale = HelperKey {
        launch: key.launch + 1,
        ..key
    };
    core.page_helper_closed(stale, "old launch ended");
    core.page_helper_line(stale, READY_LINE.as_bytes());
    assert!(!helper.0.get());
    assert!(core.library().held_session(row).unwrap().overlay.page().is_none());

    assert!(permits.take());
    core.page_helper_line(key, READY_LINE.as_bytes());
    let local = core.library().held_session(row).unwrap().overlay.page().unwrap().id;
    assert!(permits.take());
    core.page_helper_line(key, b"ignored after ready");
    assert!(!helper.0.get());
    assert_eq!(
        core.library().held_session(row).unwrap().overlay.page().unwrap().id,
        local
    );
    core.shutdown_replica_channels();
    assert!(helper.0.get());
    assert!(!permits.take());
}

#[test]
fn replica_debug_output_names_live_and_pending_pages_without_the_token() {
    let (mut core, row, launcher) = core();
    let request = page(&json!({"file": {"path": "/srv/report.html"}}));
    core.show_replica_page(&request, row).unwrap();
    let local = core.library().held_session(row).unwrap().overlay.page().unwrap().id;
    core.show_replica_page(&loopback(), row).unwrap();
    let pending = launcher.launches.borrow()[1].0.page;
    let output = format!("{:?}", core.remote.viewer.pages);
    assert!(output.contains(&local.to_string()));
    assert!(output.contains(&pending.to_string()));
    assert!(output.contains("launcher: true"));
    assert!(!output.contains("tok-1"));
    core.shutdown_replica_channels();
}

#[test]
fn requests_after_helper_exit_fail_once_before_the_page_sweep() {
    let (mut core, row, launcher) = core();
    let origin = Origin::listening(&mut core, row);
    let request = page(&json!({"file": {"path": "/srv/report.html"}}));
    core.show_replica_page(&request, row).unwrap();
    let (key, _, helper) = launcher.launches.borrow()[0].clone();
    core.page_helper_closed(key, "ssh exited");
    assert!(helper.0.get());
    assert!(core.library().held_session(row).unwrap().overlay.page().is_none());
    let fetch = Rc::new(RefCell::new(Vec::new()));
    let answers = Rc::clone(&fetch);
    core.replica_fetch(
        key.page,
        "/srv/report.html",
        Box::new(move |result| answers.borrow_mut().push(result)),
    );
    assert_eq!(*fetch.borrow(), [Err("page channel closed".to_owned())]);
    let bridge = Rc::new(RefCell::new(Vec::new()));
    let answers = Rc::clone(&bridge);
    core.replica_bridge(
        key.page,
        "{}",
        Box::new(move |result| answers.borrow_mut().push(result)),
    );
    assert_eq!(*bridge.borrow(), [Err("page channel closed".to_owned())]);
    assert!(origin.heard("page.closed").is_empty());

    // The sweep tells the origin once, and a page already released says nothing more.
    core.replica_page_released(key.page);
    core.replica_page_released(key.page);
    assert_eq!(origin.heard("page.closed"), [json!({"page": request.page})]);
    assert!(origin.heard("page.detached").is_empty());
}
