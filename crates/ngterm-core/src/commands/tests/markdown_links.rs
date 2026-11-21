//! A Markdown page that follows its local links (upstream's `ControlServerLinkOverlayTests` and
//! `ControlDispatcherOverlayTests` link-page cases): `--markdown FILE --navigation` and a clicked Markdown file open with
//! a link grant for the file's folder, one whose folder may not be followed opens without navigation, and the read-back
//! keeps the original file while naming the one shown.

use ngterm_proto::command::SessionCmd;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Fixture, args, wire};
use crate::html::markdown_page::link_grant;
use crate::html::{HtmlHost, HtmlNavigation, HtmlOverlay, HtmlPageInfo};

/// Web views that read every Markdown file.
struct Host;

impl HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }

    fn navigate(&self, _: Uuid, _: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }

    fn check_markdown(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

fn fixture(extra: usize) -> Fixture {
    let mut f = Fixture::new(extra);
    f.core.set_html_host(Box::new(Host));
    f
}

fn slot_page(f: &Fixture, id: Uuid) -> HtmlOverlay {
    f.store().session(id).unwrap().overlay.page().cloned().unwrap()
}

fn page_node(f: &mut Fixture, id: Uuid) -> Value {
    super::remote::node(f, id)["htmlOverlays"][0].clone()
}

/// A folder holding `README.md`, and that file's path.
fn readme() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("README.md");
    std::fs::write(&path, "# readme\n\n[guide](sub/guide.md)\n").unwrap();
    (dir, path.to_string_lossy().into_owned())
}

fn open_markdown(f: &mut Fixture, id: Uuid, path: &str, navigation: bool) {
    let response = f.session(
        SessionCmd::OverlayOpen,
        Some(&wire(id)),
        args(|a| {
            a.markdown_file = Some(path.to_owned());
            a.navigation = navigation.then_some(true);
        }),
    );
    assert!(response.ok, "{response:?}");
}

/// Upstream's `testAMarkdownPageOpenedWithNavigationFollowsItsLinks` at the store: the grant is the file's folder with
/// symlinks resolved, and the read-back names the original file with navigation.
#[test]
fn a_markdown_page_opened_with_navigation_carries_its_folders_grant() {
    let mut f = fixture(0);
    let id = f.sessions[0];
    let (_dir, path) = readme();
    open_markdown(&mut f, id, &path, true);
    let page = slot_page(&f, id);
    assert!(page.flags.navigation);
    assert_eq!(page.markdown_links, link_grant(&path, "/home/u"));
    assert!(page.markdown_links.is_some());
    let node = page_node(&mut f, id);
    assert_eq!(
        (node["markdown"].clone(), node["navigation"].clone()),
        (json!(path), json!(true))
    );
}

/// Without `--navigation` a Markdown page keeps its links as text.
#[test]
fn a_markdown_page_without_navigation_has_no_links() {
    let mut f = fixture(0);
    let id = f.sessions[0];
    let (_dir, path) = readme();
    open_markdown(&mut f, id, &path, false);
    let page = slot_page(&f, id);
    assert!(!page.flags.navigation && page.markdown_links.is_none());
    assert!(page_node(&mut f, id).get("navigation").is_none());
}

/// Upstream's `testAMarkdownFileInHomeOpensWithoutNavigation`: a file directly in the home folder or the file system's
/// root gets no grant, so its page opens with no navigation.
#[cfg(unix)]
#[test]
fn a_markdown_file_in_the_home_folder_or_the_root_opens_without_navigation() {
    for path in ["/home/u/README.md", "/README.md"] {
        let mut f = fixture(0);
        let id = f.sessions[0];
        open_markdown(&mut f, id, path, true);
        let page = slot_page(&f, id);
        assert!(!page.flags.navigation && page.markdown_links.is_none(), "{path}");
        assert!(page_node(&mut f, id).get("navigation").is_none(), "{path}");
    }
}

/// Upstream's `testAClickedMarkdownFileOpensWithNavigationAndAGrant`: a click asks for navigation, which the page keeps
/// with the grant read beside its file and drops without one.
#[test]
fn a_clicked_markdown_file_opens_with_navigation_only_with_its_grant() {
    let mut f = fixture(1);
    let (_dir, path) = readme();
    let grant = link_grant(&path, "/home/u");
    let (linked, plain) = (f.sessions[0], f.sessions[1]);
    assert!(f.core.open_markdown_link(path.clone(), linked, None, grant.clone()));
    assert!(f.core.open_markdown_link(path, plain, None, None));
    let page = slot_page(&f, linked);
    assert!(page.clicked && page.flags.navigation);
    assert_eq!(page.markdown_links, grant);
    let page = slot_page(&f, plain);
    assert!(page.clicked && !page.flags.navigation && page.markdown_links.is_none());
}

/// Upstream's `testALinkPageReadsBackTheFileShownAndItsHistory`: `page`, `canGoBack` and `canGoForward` follow what the
/// web view shows, `markdown` keeps the original.
#[test]
fn a_link_page_reads_back_the_file_shown_and_its_history() {
    let mut f = fixture(0);
    let id = f.sessions[0];
    let (dir, path) = readme();
    open_markdown(&mut f, id, &path, true);
    let page = slot_page(&f, id).id;
    let guide = dir.path().join("sub/guide.md").to_string_lossy().into_owned();
    let info = HtmlPageInfo {
        page: guide.clone(),
        title: None,
        can_go_back: true,
        can_go_forward: false,
    };
    assert!(f.core.set_html_page(page, Some(info)));
    let node = page_node(&mut f, id);
    assert_eq!(node["markdown"], json!(path));
    assert_eq!(node["page"], json!(guide));
    assert_eq!(
        (node["canGoBack"].clone(), node["canGoForward"].clone()),
        (json!(true), json!(false))
    );
    assert_eq!(slot_page(&f, id).identity(), "guide.md");
}
