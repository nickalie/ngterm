//! Text page wrap (upstream's `HtmlOverlayTests` wrap cases, `ControlDispatcherOverlayTests.overlayWrap*`,
//! `AppStoreRemotePageTests.wrapRefusesAPageThePresenterShows` and `ControlServerTextPageTests` wrap cases):
//! `session.overlay.wrap` sets a local text page's wrap in the model, the page's own toggle lands through the core, and
//! the tree reads it back only while the page wraps.

use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Args, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::remote_page::Origin;
use super::{Fixture, args, failure, wire};
use crate::html::text_page::{TextLog, TextLogState};
use crate::html::{HtmlHost, HtmlNavigation, HtmlOverlay, HtmlPageFlags, HtmlSource, ReplicaContent};
use crate::model::{OverlayPane, OverlayProgram, TextWrapFailure};
use crate::remote::hub::SUPPORTED_KINDS;

/// Web views that take every page; a wrap never asks them anything.
struct Host;

impl HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }
    fn navigate(&self, _: Uuid, _: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
    fn check_text(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn check_markdown(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

fn fixture() -> Fixture {
    let mut f = Fixture::new(1);
    f.core.set_html_host(Box::new(Host));
    f
}

fn path(name: &str) -> String {
    crate::html::os_path(&format!("/tmp/{name}"))
}

/// Opens `name` as a text page in session `id`'s slot `pane`; its page id.
fn open_text(f: &mut Fixture, id: Uuid, name: &str, pane: Option<&str>) -> Uuid {
    let opened = f.session(
        SessionCmd::OverlayOpen,
        Some(&wire(id)),
        args(|a| {
            a.text_file = Some(path(name));
            a.pane = pane.map(str::to_owned);
        }),
    );
    assert!(opened.ok, "{opened:?}");
    page(f, id, pane.map(|_| OverlayPane::Left)).id
}

fn page(f: &Fixture, id: Uuid, pane: Option<OverlayPane>) -> HtmlOverlay {
    f.store().html_overlay(id, pane).unwrap().clone()
}

fn wrap(f: &mut Fixture, target: &str, edit: impl FnOnce(&mut Args)) -> Response {
    f.session(SessionCmd::OverlayWrap, Some(target), args(edit))
}

fn mode(mode: &str) -> impl FnOnce(&mut Args) {
    move |a: &mut Args| a.mode = Some(mode.to_owned())
}

fn answer(id: Uuid, state: &str) -> Response {
    Response::with_result(json!({"id": wire(id), "text": state}))
}

/// The `text` node of session `id`'s session-wide page.
fn text(f: &mut Fixture, id: Uuid) -> Value {
    let tree = serde_json::to_value(f.core.tree(None).unwrap()).unwrap();
    let session = tree["workspaces"][0]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == wire(id))
        .unwrap()
        .clone();
    session["htmlOverlays"][0]["text"].clone()
}

/// Upstream's `wrapModeSetsTheTextPage`: each mode leaves the state it names, from either state, emitting once.
#[test]
fn each_mode_sets_the_text_page_and_emits_once() {
    for (raw, from, to) in [
        ("on", false, true),
        ("off", true, false),
        ("toggle", true, false),
        ("toggle", false, true),
    ] {
        let mut f = fixture();
        let id = f.sessions[0];
        let page_id = open_text(&mut f, id, "a.txt", None);
        f.core.library_mut().set_text_wrap(page_id, from);
        f.core.commit();

        let state = if to { "wrapped" } else { "unwrapped" };
        assert_eq!(
            wrap(&mut f, &wire(id), mode(raw)),
            answer(id, state),
            "{raw} from {from}"
        );

        assert_eq!(page(&f, id, None).text_wrap, to, "{raw} from {from}");
        assert_eq!(f.core.commit().tree_changed.len(), 1, "{raw} from {from}");
    }
}

/// Upstream's `overlayWrapDefaultsToToggle` and `aWrapThatChangesNothingEmitsNothing`: an absent mode toggles, and a
/// wrap that leaves the state as it was emits nothing.
#[test]
fn an_absent_mode_toggles_and_no_change_emits_nothing() {
    let mut f = fixture();
    let id = f.sessions[0];
    let page_id = open_text(&mut f, id, "a.log", None);
    f.core.commit();
    assert_eq!(wrap(&mut f, &wire(id), |_| {}), answer(id, "wrapped"));
    assert_eq!(f.core.commit().tree_changed.len(), 1);

    assert!(!f.core.library_mut().set_text_wrap(page_id, true));
    assert_eq!(wrap(&mut f, &wire(id), mode("on")), answer(id, "wrapped"));

    assert!(page(&f, id, None).text_wrap);
    assert!(f.core.commit().tree_changed.is_empty());
}

/// Upstream's `overlayWrapRoutesItsModeAndPane`: `--pane` addresses that pane's page and leaves the session's alone.
#[test]
fn a_pane_wrap_reaches_that_panes_page() {
    let mut f = fixture();
    let id = f.sessions[0];
    open_text(&mut f, id, "a.txt", Some("left"));
    let on_left = |a: &mut Args| {
        a.mode = Some("on".to_owned());
        a.pane = Some("left".to_owned());
    };
    assert_eq!(wrap(&mut f, &wire(id), on_left), answer(id, "wrapped"));
    assert!(page(&f, id, Some(OverlayPane::Left)).text_wrap);
    assert_eq!(wrap(&mut f, &wire(id), mode("on")), failure("no overlay"));
}

/// Upstream's `wrapRefusesEverythingButATextPage`: an empty slot, a Markdown page, a page shown for another machine
/// and a program are refused, and a wrap never lands on a page that is not a text page.
#[test]
fn a_wrap_addresses_only_a_local_text_page() {
    let mut f = fixture();
    let id = f.sessions[0];
    let target = wire(id);
    assert_eq!(wrap(&mut f, &target, mode("on")), failure("no overlay"));
    assert!(
        f.session(
            SessionCmd::OverlayOpen,
            Some(&target),
            args(|a| a.markdown_file = Some(path("a.md")))
        )
        .ok
    );
    assert_eq!(wrap(&mut f, &target, mode("on")), failure("no text page"));
    let markdown = page(&f, id, None).id;
    assert!(!f.core.library_mut().set_text_wrap(markdown, true));
    assert!(!page(&f, id, None).text_wrap);
    assert!(f.store_mut().unwrap().close_overlay(id));

    assert!(f.store_mut().unwrap().open_overlay(
        id,
        OverlayProgram {
            command: "less".to_owned(),
            ..OverlayProgram::default()
        }
    ));
    assert_eq!(
        wrap(&mut f, &target, mode("on")),
        failure("the overlay is not an html page")
    );
    assert_eq!(TextWrapFailure::NotText.message(), "no text page");
    assert_eq!(
        TextWrapFailure::Remote.message(),
        "remote page: wrap it on the Mac that shows it"
    );
}

/// Upstream's `setTextWrap` on a replica (#978): a text page shown here for another machine wraps as a local one, from
/// control and from its own toggle.
#[test]
fn a_text_replica_wraps_as_a_local_text_page() {
    let mut f = fixture();
    let id = f.sessions[0];
    let target = wire(id);
    let replica = HtmlOverlay {
        replica_of: Some(Uuid::new_v4()),
        ..HtmlOverlay::new(
            HtmlSource::Replica {
                path: "/tmp/a.txt".to_owned(),
                content: ReplicaContent::Text { line: None },
            },
            HtmlPageFlags::default(),
        )
    };
    let replica_id = replica.id;
    f.store_mut()
        .unwrap()
        .open_html_overlay(id, None, replica, None, None)
        .unwrap();
    assert_eq!(wrap(&mut f, &target, mode("on")).result.unwrap()["text"], "wrapped");
    assert!(!f.core.set_text_wrap_from_page(replica_id, true));
    assert!(f.core.set_text_wrap_from_page(replica_id, false));
    assert!(!page(&f, id, None).text_wrap);
    assert!(f.store_mut().unwrap().close_overlay(id));
}

/// Upstream's `overlayWrapRejectsBadInputBeforeCallingActions`: the mode, then the pane, before the session resolves.
#[test]
fn a_wrap_refuses_bad_input_before_resolving_a_target() {
    let mut f = fixture();
    for (edit, error) in [
        (args(mode("maybe")), "invalid overlay wrap mode: maybe"),
        (
            args(|a| {
                a.mode = Some("on".to_owned());
                a.pane = Some("middle".to_owned());
            }),
            "session.overlay: --pane must be left or right",
        ),
        (args(mode("on")), "no such session: missing"),
    ] {
        assert_eq!(
            f.session(SessionCmd::OverlayWrap, Some("missing"), edit),
            failure(error)
        );
    }
}

/// Upstream's `aTextPageNodeReportsWrapOnlyWhileWrapped`, with the page's own toggle (upstream's
/// `TextPageBridge.wrapped`) writing the same model: a log page too, and the read-back omits `wrap` while unwrapped.
#[test]
fn the_pages_own_toggle_writes_the_model_and_the_tree_reads_it_only_while_wrapped() {
    let mut f = fixture();
    let id = f.sessions[0];
    let log = TextLog { tail: 10, follow: true };
    let log_page = HtmlOverlay {
        text_log: Some(log),
        log_state: Some(TextLogState::new(log)),
        ..HtmlOverlay::new(
            HtmlSource::Text {
                path: path("a.log"),
                line: None,
            },
            HtmlPageFlags::default(),
        )
    };
    let page_id = log_page.id;
    f.store_mut()
        .unwrap()
        .open_html_overlay(id, None, log_page, None, None)
        .unwrap();
    f.core.commit();
    assert!(text(&mut f, id).get("wrap").is_none());

    assert!(f.core.set_text_wrap_from_page(page_id, true));
    assert!(!f.core.set_text_wrap_from_page(page_id, true));
    assert_eq!(f.core.commit().tree_changed.len(), 1);
    assert_eq!(text(&mut f, id)["wrap"], json!(true));

    assert!(f.core.set_text_wrap_from_page(page_id, false));
    assert!(text(&mut f, id).get("wrap").is_none());
}

/// Upstream's `wrapRefusesAPageThePresenterShows`: a slot the placement rule hands to the presenter is the presenter's
/// to wrap.
#[test]
fn a_presented_slot_is_wrapped_where_it_is_shown() {
    let mut origin = Origin::build(false, &SUPPORTED_KINDS, false);
    assert_eq!(
        origin.overlay(SessionCmd::OverlayWrap, mode("on")),
        failure("remote page: wrap it on the Mac that shows it")
    );
}
