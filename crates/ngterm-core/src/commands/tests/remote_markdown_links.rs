//! A Markdown link page handed to the viewer presenting a session, on the origin (upstream's
//! `AppStoreRemotePageTests.aLinkGrantReachesOnlyAPresenterThatFollowsLinks` and
//! `ControlServerForwardedClickTests.testAClickedMarkdownFileReachesThePresenterWithNavigationAndItsLinkGrant`, #931):
//! the grant rides the request only to a presenter listing `page.links`, and any other gets the page without links, in
//! the request and in the record its channel serves from, a parked page handed on included.

use serde_json::{Value, json};

use super::content_pages::{Files, content_origin, open_content};
use super::remote_page::Origin;
use crate::html::HtmlOverlay;
use crate::html::markdown_page::{MarkdownLinkGrant, link_grant};
use crate::remote::hub::{PAGE_LINKS_KIND, SUPPORTED_KINDS};

fn kinds(follows: bool) -> Vec<&'static str> {
    SUPPORTED_KINDS
        .into_iter()
        .filter(|kind| follows || *kind != PAGE_LINKS_KIND)
        .collect()
}

fn request(origin: &Origin) -> Value {
    origin.presenter.last("page.request").unwrap()["page"].clone()
}

/// The page the session-wide slot holds for the presenter.
fn recorded(origin: &mut Origin) -> HtmlOverlay {
    let session = origin.session();
    let found = origin.fixture().store().session(session).unwrap();
    let slot = found.remote_overlays().slot(None).unwrap();
    slot.page.as_ref().unwrap().overlay.clone()
}

fn wire(grant: &MarkdownLinkGrant) -> Value {
    json!({"path": grant.path, "root": grant.root})
}

/// Opens the folder's `README.md` with `--navigation`; the grant its open froze.
fn open_linked(origin: &mut Origin, files: &Files) -> MarkdownLinkGrant {
    let path = files.path("README.md");
    let response = open_content(origin, "markdown", &path, |a| a.navigation = Some(true));
    assert!(response.ok, "{response:?}");
    link_grant(&path, "/home/u").unwrap()
}

/// Upstream's `aLinkGrantReachesOnlyAPresenterThatFollowsLinks`: the request and the record agree, and keep the page's
/// id.
#[test]
fn a_link_grant_reaches_only_a_presenter_that_follows_links() {
    let files = Files::new();
    for follows in [true, false] {
        let mut origin = content_origin(&kinds(follows));
        let grant = open_linked(&mut origin, &files);
        let sent = request(&origin);
        assert_eq!(
            sent.get("markdownLinks").cloned(),
            follows.then(|| wire(&grant)),
            "{follows}"
        );
        assert_eq!(sent["navigation"], json!(follows), "{follows}");
        let page = recorded(&mut origin);
        assert_eq!(page.markdown_links, follows.then(|| grant.clone()), "{follows}");
        assert_eq!(page.flags.navigation, follows, "{follows}");
        assert_eq!(sent["page"], json!(crate::pane_env::uuid_string(page.id)), "{follows}");
    }
}

/// A page with no grant sends no grant key, whatever the presenter lists.
#[test]
fn a_page_without_a_grant_sends_no_grant_key() {
    let files = Files::new();
    let mut origin = content_origin(&SUPPORTED_KINDS);
    assert!(open_content(&mut origin, "markdown", &files.path("README.md"), |_| {}).ok);
    let sent = request(&origin);
    assert!(sent.get("markdownLinks").is_none(), "{sent}");
    assert_eq!(sent["navigation"], json!(false));
}

/// Upstream's `testAClickedMarkdownFileReachesThePresenterWithNavigationAndItsLinkGrant`: a clicked Markdown file goes
/// with navigation and the grant read beside it.
#[test]
fn a_clicked_markdown_file_reaches_the_presenter_with_navigation_and_its_grant() {
    let files = Files::new();
    let path = files.path("README.md");
    let grant = link_grant(&path, "/home/u");
    assert!(grant.is_some());
    let mut origin = content_origin(&SUPPORTED_KINDS);
    let session = origin.session();
    assert!(
        origin
            .fixture()
            .core
            .open_markdown_link(path, session, None, grant.clone())
    );
    let sent = request(&origin);
    assert_eq!(sent["navigation"], json!(true));
    assert_eq!(sent.get("markdownLinks").cloned(), grant.as_ref().map(wire));
}

/// A parked link page handed on to a presenter that cannot follow links goes, and is served from then on, without its
/// grant; a later presenter that can gets no grant back.
#[test]
fn a_parked_link_page_handed_on_without_links_loses_its_grant() {
    let files = Files::new();
    let mut origin = content_origin(&SUPPORTED_KINDS);
    let grant = open_linked(&mut origin, &files);
    let page = request(&origin)["page"].clone();
    origin.lose_presenter();

    origin.reattach(&kinds(false));
    let handed = request(&origin);
    assert_eq!(handed["page"], page);
    assert!(handed.get("markdownLinks").is_none(), "{handed}");
    assert_eq!(handed["navigation"], json!(false));
    let recorded_page = recorded(&mut origin);
    assert!(recorded_page.markdown_links.is_none() && !recorded_page.flags.navigation);
    origin.lose_presenter();

    origin.reattach(&SUPPORTED_KINDS);
    let handed = request(&origin);
    assert_eq!(handed["page"], page);
    assert!(handed.get("markdownLinks").is_none(), "{handed}");
    assert_ne!(Some(grant), recorded(&mut origin).markdown_links);
}

/// A parked link page handed on to a presenter that follows links keeps its grant.
#[test]
fn a_parked_link_page_handed_on_with_links_keeps_its_grant() {
    let files = Files::new();
    let mut origin = content_origin(&SUPPORTED_KINDS);
    let grant = open_linked(&mut origin, &files);
    origin.lose_presenter();
    origin.reattach(&SUPPORTED_KINDS);
    let handed = request(&origin);
    assert_eq!(handed.get("markdownLinks").cloned(), Some(wire(&grant)));
    assert_eq!(handed["navigation"], json!(true));
    assert_eq!(recorded(&mut origin).markdown_links, Some(grant));
}
