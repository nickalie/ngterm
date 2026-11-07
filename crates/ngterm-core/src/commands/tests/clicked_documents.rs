//! A clicked HTML file's page (upstream's `ControlServer+DocumentPage`, `AppStoreTreeProjectionTests.
//! aDocumentPageProjectsOnlyItsDocumentSource`, `ControlServerLinkOverlayTests.
//! testAClickedHtmlFileOpensAsADocumentAndAnUnreadableOneOpensNothing`'s store half).

use ngterm_proto::command::SessionCmd;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Fixture, args, failure, wire};
use crate::html::{HtmlNavigation, HtmlSource, errors};

struct Host;

impl crate::html::HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }
    fn navigate(&self, _: Uuid, _: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
}

fn fixture() -> Fixture {
    let mut f = Fixture::new(1);
    f.core.set_html_host(Box::new(Host));
    f
}

fn document() -> HtmlSource {
    HtmlSource::Document {
        path: crate::html::os_path("/tmp/agdoc/pr.html"),
    }
}

fn page(f: &mut Fixture, id: Uuid) -> Option<Value> {
    let tree = serde_json::to_value(f.core.tree(None).unwrap()).unwrap();
    tree["workspaces"][0]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == wire(id))
        .and_then(|node| node.get("htmlOverlays"))
        .map(|pages| pages[0].clone())
}

/// Upstream's `aDocumentPageProjectsOnlyItsDocumentSource`: the read-back names the file as `document` alone, between
/// `markdown` and `text`, with page JavaScript off.
#[test]
fn a_document_page_projects_only_its_document_source() {
    let mut f = fixture();
    let id = f.sessions[0];
    assert!(f.core.open_document_link(document(), id, Some(95), None));
    let node = page(&mut f, id).unwrap();
    let path = crate::html::os_path("/tmp/agdoc/pr.html");
    assert_eq!(node["document"], json!(path));
    for absent in ["file", "cwd", "url", "image", "markdown", "text"] {
        assert!(node.get(absent).is_none(), "{absent} in {node}");
    }
    assert_eq!(node["javascript"], json!(false));
    let keys: Vec<&str> = node.as_object().unwrap().keys().map(String::as_str).collect();
    let at = |key: &str| keys.iter().position(|found| *found == key);
    assert!(at("pane").is_none_or(|pane| pane < at("document").unwrap()));
    assert!(at("document").unwrap() < at("state").unwrap(), "{keys:?}");
}

/// A clicked document opens on its session at the click's size without selecting it, and a busy slot refuses a second.
#[test]
fn a_document_click_opens_at_its_size_without_selecting_its_session() {
    let mut f = fixture();
    let id = f.sessions[0];
    let selected = f.store().selected_session_id();
    assert!(f.core.open_document_link(document(), id, Some(75), None));
    assert!(
        !f.core.open_document_link(document(), id, Some(75), None),
        "a busy slot"
    );
    assert_eq!(f.store().selected_session_id(), selected);
    {
        let session = f.store().session(id).unwrap();
        let shown = session.html_overlays();
        let (_, overlay) = shown.first().unwrap();
        assert_eq!(overlay.source, document());
        assert!(!overlay.flags.javascript && overlay.blocks_network());
    }
    let tree = serde_json::to_value(f.core.tree(None).unwrap()).unwrap();
    assert_eq!(tree["workspaces"][0]["sessions"][0]["overlaySizePercent"], json!(75));
}

/// Upstream's `documentInOverlay`: the browser would run the file's scripts, and the file manager reveals file pages
/// only.
#[test]
fn a_document_page_refuses_the_browser() {
    let mut f = fixture();
    let id = f.sessions[0];
    assert!(f.core.open_document_link(document(), id, None, None));
    let browser = f.session(
        SessionCmd::OverlayNavigate,
        Some(&wire(id)),
        args(|a| a.to = Some("browser".to_owned())),
    );
    assert_eq!(browser, failure(errors::DOCUMENT_IN_OVERLAY));
}

/// Until a presenter can build one (Task 11), no presenter is handed a document page.
#[test]
fn no_presenter_shows_a_document_page() {
    assert!(crate::remote::presentation_source(&document()).is_none());
}
