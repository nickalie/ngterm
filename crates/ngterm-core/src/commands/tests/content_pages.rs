//! Image, Markdown, text and document pages handed to the viewer presenting a session, on the origin (upstream's
//! `AppStoreRemotePageTests` content cases, `ControlServerPageServeTests.testAnImagePageTravelsWithItsPixelsAndNoOriginSize`
//! and the hand-over half of `ControlServerRemotePresentationTests`, #891 and #917): the probe before the hand-over,
//! the capability gate per kind, the presenter's own sizing of a picture and the merging resize it takes, the claim
//! expiry, the navigation refusals, the hand-on of a parked content page, and a headless origin's probe.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ngterm_proto::command::SessionCmd;
use serde_json::{Value, json};
use uuid::Uuid;

use super::failure;
use super::remote::set_lead;
use super::remote_page::Origin;
use crate::html::image_page::{BASE_PERCENT, ImageData};
use crate::html::{HtmlHost, HtmlNavigation, HtmlSource, PageProbe, image_page};
use crate::model::{OverlayPane, OverlayPointSize};
use crate::remote::hub::{PAGE_CONTENT_KIND, PAGE_DOCUMENT_KIND, SUPPORTED_KINDS};
use crate::remote::{PAGE_CLAIM_WINDOW, RemoteOverlayResize};

const PRESENTER: &str = "a viewer presents this session: an html overlay would open where nobody sees it";
const REMOTE_PAGE: &str = "page is shown on another Mac";

/// A picture's first bytes: the probes here go by the signature, every picture 640x480.
pub(super) const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake image data";

/// Reads a page's file as the app's probes would, without decoding: a picture is a file whose bytes sniff as one, sized
/// 640x480; a Markdown or text file any regular file. The web views can run.
pub(super) struct ContentHost;

/// The probes [`ContentHost`] and a headless origin's [`PageProbe`] share.
fn probe_image(path: &str) -> Result<ImageData, String> {
    let bytes = std::fs::read(path).map_err(|_| format!("unreadable image: {path}"))?;
    let mime_type =
        image_page::mime_type(&bytes).ok_or_else(|| format!("not a png, jpg, gif or webp image: {path}"))?;
    Ok(ImageData {
        bytes,
        mime_type: mime_type.to_owned(),
        width: 640,
        height: 480,
        orientation: 1,
    })
}

fn check(path: &str, kind: &str) -> Result<(), String> {
    Path::new(path)
        .is_file()
        .then_some(())
        .ok_or_else(|| format!("unreadable {kind} file: {path}"))
}

impl HtmlHost for ContentHost {
    fn probe_image(&self, path: &str) -> Result<ImageData, String> {
        probe_image(path)
    }

    fn check_markdown(&self, path: &str) -> Result<(), String> {
        check(path, "markdown")
    }

    fn check_text(&self, path: &str) -> Result<(), String> {
        check(path, "text")
    }

    fn display_scale(&self, _: Uuid) -> f64 {
        2.0
    }

    fn persistent_store_failure(&self) -> Option<String> {
        None
    }

    fn navigate(&self, _: Uuid, _: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
}

/// A headless origin's probe: no web views.
struct Probe;

impl PageProbe for Probe {
    fn probe_image(&self, path: &str) -> Result<ImageData, String> {
        probe_image(path)
    }

    fn check_markdown(&self, path: &str) -> Result<(), String> {
        check(path, "markdown")
    }

    fn check_text(&self, path: &str) -> Result<(), String> {
        check(path, "text")
    }

    fn check_log(&self, path: &str, _: usize) -> Result<(), String> {
        check(path, "text")
    }
}

/// A folder of content files.
pub(super) struct Files {
    _dir: tempfile::TempDir,
    pub(super) root: PathBuf,
}

impl Files {
    pub(super) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap().join("docs");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("shot.png"), PNG).unwrap();
        std::fs::write(root.join("README.md"), "# notes\n\n![s](shot.png)\n").unwrap();
        std::fs::write(root.join("main.go"), "package main\n").unwrap();
        std::fs::write(root.join("pr.html"), "<p>review</p>").unwrap();
        Self { _dir: dir, root }
    }

    pub(super) fn path(&self, name: &str) -> String {
        self.root.join(name).to_str().unwrap().to_owned()
    }
}

/// A presented session whose presenter lists `kinds`, its origin's web views reading content files.
pub(super) fn content_origin(kinds: &[&str]) -> Origin {
    let mut origin = Origin::build(true, kinds, false);
    origin.fixture().core.set_html_host(Box::new(ContentHost));
    origin
}

/// Every kind this build speaks but those `without` names.
fn kinds_without(without: &[&str]) -> Vec<&'static str> {
    SUPPORTED_KINDS
        .into_iter()
        .filter(|kind| !without.contains(kind))
        .collect()
}

/// Opens `--image`, `--markdown` or `--text` `path` with `edit` applied.
pub(super) fn open_content(
    origin: &mut Origin,
    flag: &str,
    path: &str,
    edit: impl FnOnce(&mut ngterm_proto::Args),
) -> ngterm_proto::Response {
    let path = path.to_owned();
    let flag = flag.to_owned();
    origin.overlay(SessionCmd::OverlayOpen, move |a| {
        match flag.as_str() {
            "image" => a.image = Some(path),
            "markdown" => a.markdown_file = Some(path),
            _ => a.text_file = Some(path),
        }
        edit(a);
    })
}

fn request(origin: &Origin) -> Value {
    origin.presenter.last("page.request").unwrap()["page"].clone()
}

fn page_requests(origin: &Origin) -> usize {
    origin.presenter.frames("page.request").len()
}

/// Upstream's `aContentPageOpensOnAPresenterThatBuildsIt` and `testAnImagePageTravelsWithItsPixelsAndNoOriginSize`:
/// each kind goes with its source, unsized; nothing covers the session here; a picture travels with its pixels and
/// its slot records the base percent the presenter fits it at, so the tree reads it floating.
#[test]
fn a_content_page_opens_on_a_presenter_that_builds_it() {
    let files = Files::new();
    for (flag, name, sent) in [
        (
            "image",
            "shot.png",
            json!({"image": {"path": files.path("shot.png"), "width": 640, "height": 480}}),
        ),
        (
            "markdown",
            "README.md",
            json!({"markdown": {"path": files.path("README.md")}}),
        ),
        (
            "text",
            "main.go",
            json!({"text": {"path": files.path("main.go"), "line": 3}}),
        ),
    ] {
        let mut origin = content_origin(&SUPPORTED_KINDS);
        let response = open_content(&mut origin, flag, &files.path(name), |a| {
            a.line = (flag == "text").then_some(3);
        });
        assert!(response.ok, "{flag}: {response:?}");
        let page = request(&origin);
        assert_eq!(page["source"], sent, "{flag}");
        assert!(
            page.get("sizePercent").is_none() && page.get("width").is_none(),
            "{flag}: {page}"
        );
        let session = origin.live.fixture.store().session(origin.session()).unwrap();
        assert!(!session.overlay.active, "{flag}: nothing covers the session here");
        let slot = session.remote_overlays().slot(None).unwrap();
        let fitted = flag == "image";
        assert_eq!(slot.size_percent, fitted.then_some(BASE_PERCENT), "{flag}");
        assert_eq!(slot.presenter_sized, fitted, "{flag}");
        let id = origin.session();
        let tree = super::remote::node(&mut origin.live.fixture, id);
        let reserved = &tree["remoteOverlays"][0];
        assert_eq!(
            reserved.get("sizePercent").cloned(),
            fitted.then(|| json!(BASE_PERCENT)),
            "{flag}"
        );
    }
}

/// Upstream's `aPresenterWithoutContentPagesKeepsTheRefusal` and `testAPresenterWithoutContentPagesRefusesThemAnd
/// StillTakesAFilePage`; a clicked document needs `page.document` too (#917).
#[test]
fn a_presenter_without_content_pages_keeps_the_refusal() {
    let files = Files::new();
    let older = kinds_without(&[PAGE_CONTENT_KIND, PAGE_DOCUMENT_KIND]);
    for (flag, name) in [("image", "shot.png"), ("markdown", "README.md"), ("text", "main.go")] {
        let mut origin = content_origin(&older);
        assert_eq!(
            open_content(&mut origin, flag, &files.path(name), |_| {}),
            failure(PRESENTER),
            "{flag}"
        );
        let session = origin.live.fixture.store().session(origin.session()).unwrap();
        assert!(
            session.remote_overlays().slots().is_empty() && !session.overlay.active,
            "{flag}"
        );
        assert_eq!(page_requests(&origin), 0, "{flag}");
        assert!(
            origin.opened("/tmp/docs/index.html", |_| {}).len() == 36,
            "a file page still goes"
        );
    }
    let document = HtmlSource::Document {
        path: files.path("pr.html"),
    };
    for (kinds, shown) in [
        (kinds_without(&[PAGE_DOCUMENT_KIND]), false),
        (SUPPORTED_KINDS.to_vec(), true),
    ] {
        let mut origin = content_origin(&kinds);
        let session = origin.session();
        assert_eq!(
            origin
                .fixture()
                .core
                .open_document_link(document.clone(), session, None, None),
            shown
        );
        assert_eq!(page_requests(&origin), usize::from(shown));
        if shown {
            assert_eq!(
                request(&origin)["source"],
                json!({"document": {"path": files.path("pr.html")}})
            );
        }
    }
}

/// The control path reads the file before anything is reserved (upstream's `openSessionOverlay` order): a missing
/// picture, Markdown or text file is refused with its probe's reason, and the presenter hears nothing.
#[test]
fn a_content_page_is_probed_before_it_is_handed_over() {
    let files = Files::new();
    let mut origin = content_origin(&SUPPORTED_KINDS);
    for (flag, name, error) in [
        ("image", "missing.png", "unreadable image"),
        ("markdown", "missing.md", "unreadable markdown file"),
        ("text", "missing.go", "unreadable text file"),
    ] {
        assert_eq!(
            open_content(&mut origin, flag, &files.path(name), |_| {}),
            failure(&format!("session.overlay.open: {error}: {}", files.path(name)))
        );
    }
    assert_eq!(page_requests(&origin), 0);
    let session = origin.live.fixture.store().session(origin.session()).unwrap();
    assert!(session.remote_overlays().slots().is_empty());
}

/// Upstream's `onlyAnUnsizedSessionWideImageTakesTheBasePercent`.
#[test]
fn only_an_unsized_session_wide_image_takes_the_base_percent() {
    let files = Files::new();
    for (flag, name, pane, size, reserved) in [
        ("image", "shot.png", Some(OverlayPane::Left), None, None),
        ("image", "shot.png", None, Some(60), Some(60)),
        ("markdown", "README.md", None, None, None),
    ] {
        let mut origin = content_origin(&SUPPORTED_KINDS);
        let response = open_content(&mut origin, flag, &files.path(name), |a| {
            a.pane = pane.map(|pane| pane.wire_name().to_owned());
            a.size_percent = size;
        });
        assert!(response.ok, "{response:?}");
        let session = origin.live.fixture.store().session(origin.session()).unwrap();
        let slot = session.remote_overlays().slot(pane).unwrap();
        assert_eq!(
            (slot.size_percent, slot.presenter_sized),
            (reserved, false),
            "{flag} {pane:?} {size:?}"
        );
    }
}

fn last_command(origin: &Origin) -> Value {
    origin.presenter.last("page.command").unwrap()["page"].clone()
}

/// Upstream's `anAutoSizedImageReservesTheBasePercentAndTakesAPointResize` and `pointResizesOfAPresenterSizedImage
/// SendOnlyTheirAxesToMergeUntilASizeReplacesIt`.
#[test]
fn point_resizes_of_a_presenter_sized_image_merge_until_a_size_replaces_it() {
    let files = Files::new();
    let mut origin = content_origin(&SUPPORTED_KINDS);
    assert!(open_content(&mut origin, "image", &files.path("shot.png"), |_| {}).ok);
    let resize = |origin: &mut Origin, size: Option<i64>, width: Option<i64>, height: Option<i64>| {
        let response = origin.overlay(SessionCmd::OverlayResize, |a| {
            a.size_percent = size;
            a.width = width;
            a.height = height;
        });
        assert!(response.ok, "{response:?}");
        let mut command = last_command(origin);
        command.as_object_mut().unwrap().remove("page");
        command.as_object_mut().unwrap().remove("seq");
        command
    };
    assert_eq!(
        resize(&mut origin, None, Some(600), None),
        json!({"action": "resize", "sizePercent": BASE_PERCENT, "width": 600, "merge": true})
    );
    assert_eq!(
        resize(&mut origin, None, None, Some(300)),
        json!({"action": "resize", "sizePercent": BASE_PERCENT, "height": 300, "merge": true})
    );
    let slot = |origin: &Origin| {
        let session = origin.live.fixture.store().session(origin.session()).unwrap();
        session.remote_overlays().slot(None).unwrap().clone()
    };
    assert_eq!(
        slot(&origin).points,
        OverlayPointSize {
            width: Some(600),
            height: Some(300)
        }
    );
    assert_eq!(
        resize(&mut origin, Some(60), Some(500), None),
        json!({"action": "resize", "sizePercent": 60, "width": 500})
    );
    assert!(!slot(&origin).presenter_sized);
    assert_eq!(
        resize(&mut origin, None, None, Some(200)),
        json!({"action": "resize", "sizePercent": 60, "width": 500, "height": 200})
    );
}

/// Upstream's `aFullResizeEndsThePresenterSizing` and `aSizedImageAndOtherPagesNeverAskToMerge`.
#[test]
fn a_full_resize_ends_the_presenter_sizing_and_a_sized_image_never_merges() {
    let files = Files::new();
    let mut origin = content_origin(&SUPPORTED_KINDS);
    assert!(open_content(&mut origin, "image", &files.path("shot.png"), |_| {}).ok);
    let session = origin.session();
    let core = &mut origin.fixture().core;
    assert_eq!(
        core.resize_remote_overlay(session, None, OverlayPointSize::default()),
        Some(RemoteOverlayResize::Sent)
    );
    assert!(last_command(&origin).get("merge").is_none());
    let core = &mut origin.fixture().core;
    let width = OverlayPointSize {
        width: Some(600),
        height: None,
    };
    assert_eq!(
        core.resize_remote_overlay(session, None, width),
        Some(RemoteOverlayResize::NotFloating)
    );

    let mut sized = content_origin(&SUPPORTED_KINDS);
    assert!(
        open_content(&mut sized, "image", &files.path("shot.png"), |a| a.size_percent =
            Some(70))
        .ok
    );
    assert!(sized.overlay(SessionCmd::OverlayResize, |a| a.width = Some(600)).ok);
    let command = last_command(&sized);
    assert!(command.get("merge").is_none());
    assert_eq!(command["sizePercent"], 70);
}

/// Upstream's `aContentPageRefusesFinderAndBrowserLikeAFilePage`.
#[test]
fn a_content_page_refuses_the_finder_and_the_browser_like_a_file_page() {
    let files = Files::new();
    let mut origin = content_origin(&SUPPORTED_KINDS);
    assert!(open_content(&mut origin, "markdown", &files.path("README.md"), |_| {}).ok);
    for to in ["browser", "finder"] {
        let response = origin.overlay(SessionCmd::OverlayNavigate, |a| a.to = Some(to.to_owned()));
        assert_eq!(response, failure(REMOTE_PAGE), "{to}");
    }
    assert!(origin.presenter.frames("page.command").is_empty());
}

/// Upstream's `expiryEndsAnUnclaimedContentPage`, but the expiry parks the page for the next presenter, as a file
/// page's does here (an extension beyond upstream).
#[test]
fn an_unclaimed_content_page_expires_like_a_file_page() {
    let files = Files::new();
    let mut origin = content_origin(&SUPPORTED_KINDS);
    assert!(open_content(&mut origin, "text", &files.path("main.go"), |_| {}).ok);
    let core = &mut origin.fixture().core;
    assert_eq!(
        core.take_page_claim_expiries().len(),
        1,
        "a channel page arms the expiry"
    );
    core.expire_remote_page_claims_at(Instant::now() + PAGE_CLAIM_WINDOW + Duration::from_secs(1));
    core.commit();
    let session = origin.live.fixture.store().session(origin.session()).unwrap();
    let parked = session.remote_overlays().slot(None).unwrap().page.as_ref().unwrap();
    assert!(parked.parked);
    assert_eq!(origin.presenter.frames("page.close").len(), 1);
}

/// A content page parked while no presenter builds it waits for one that does, and a picture a restore parked again
/// with no probed size is read when a presenter takes it.
#[test]
fn a_parked_content_page_goes_only_to_a_presenter_that_builds_it() {
    let files = Files::new();
    let mut origin = content_origin(&SUPPORTED_KINDS);
    assert!(open_content(&mut origin, "image", &files.path("shot.png"), |_| {}).ok);
    let session = origin.session();
    let picture = request(&origin)["page"].clone();
    // the presenter goes, and the picture's probed size with the slot's record (a restore keeps neither)
    let serial = origin.serial;
    let core = &mut origin.fixture().core;
    core.presentation_closed(serial);
    core.commit();
    let store = origin.fixture().store_mut().unwrap();
    let slots = store.session_mut(session).unwrap().remote_overlays.slots_mut();
    slots[0].page.as_mut().unwrap().image_pixels = None;

    let (serial, older) = super::remote::subscribe_listing(
        &mut origin.fixture().core,
        session,
        "presenter",
        &kinds_without(&[PAGE_CONTENT_KIND]),
    );
    acquire(&mut origin, serial, &older);
    assert!(
        older.frames("page.request").is_empty(),
        "an older presenter keeps nothing it cannot build"
    );
    origin.fixture().core.presentation_closed(serial);
    origin.fixture().core.commit();

    let (serial, newer) =
        super::remote::subscribe_listing(&mut origin.fixture().core, session, "presenter", &SUPPORTED_KINDS);
    acquire(&mut origin, serial, &newer);
    let handed = newer.last("page.request").unwrap()["page"].clone();
    assert_eq!(handed["page"], picture);
    assert_eq!(
        handed["source"],
        json!({"image": {"path": files.path("shot.png"), "width": 640, "height": 480}})
    );
    assert!(handed.get("sizePercent").is_none(), "the next presenter fits it again");
}

fn acquire(origin: &mut Origin, serial: u64, presenter: &super::remote::Viewer) {
    let acquire = super::remote::frame(presenter, &json!({"kind": "presenter.acquire"}));
    origin.fixture().core.presentation_line(serial, &acquire);
    origin.fixture().core.commit();
}

/// A headless origin has no web views: its own probe reads a page's file before the hand-over, a picture's size with
/// it, and a log page is refused after its probe as it is with web views (upstream's `textLogRefusal`).
#[test]
fn a_headless_origin_probes_a_content_page_before_handing_it_over() {
    let files = Files::new();
    let mut origin = Origin::build(false, &SUPPORTED_KINDS, false);
    origin.fixture().core.set_page_presenter_only();
    let left = origin.live.left;
    set_lead(&mut origin.fixture().core, left, "leader");
    assert_eq!(
        open_content(&mut origin, "image", &files.path("shot.png"), |_| {}),
        failure("session.overlay.open: html overlays are not available in this build"),
        "nothing reads a picture here without a probe"
    );
    origin.fixture().core.set_page_probe(Box::new(Probe));
    assert!(open_content(&mut origin, "image", &files.path("shot.png"), |_| {}).ok);
    assert_eq!(request(&origin)["source"]["image"]["width"], 640);
    assert!(origin.overlay(SessionCmd::OverlayClose, |_| {}).ok);
    assert_eq!(
        open_content(&mut origin, "text", &files.path("missing.go"), |_| {}),
        failure(&format!(
            "session.overlay.open: unreadable text file: {}",
            files.path("missing.go")
        ))
    );
    assert_eq!(
        open_content(&mut origin, "text", &files.path("main.go"), |a| a.follow_file =
            Some(true)),
        failure("session.overlay.open: a log page is not shown on another Mac")
    );
}
