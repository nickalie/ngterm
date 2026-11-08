//! Diff page changes (upstream's `HtmlOverlayTests` changes cases, `ControlDispatcherOverlayTests.overlayChanges*` and
//! `ControlServerTextPageTests.testAChangesStepNeedsALoadedDiffPage`): `session.overlay.changes` steps a plain local
//! diff page through its renderer, and the page's report reads back under `text.changes`, never emitted.

use std::cell::RefCell;
use std::rc::Rc;

use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Args, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::remote_page::Origin;
use super::{Fixture, args, failure, wire};
use crate::html::text_page::{TextChanges, TextLog, TextLogState};
use crate::html::{ChangesAnswer, HtmlHost, HtmlNavigation, HtmlOverlay, HtmlPageFlags, HtmlSource};
use crate::model::{OverlayPane, OverlayProgram, TextChangesFailure};
use crate::remote::hub::SUPPORTED_KINDS;

/// How the fake page answers a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    /// Its renderer is not up: no channel.
    NotUp,
    /// The channel closes unanswered.
    Gone,
    Changes(TextChanges),
}

struct Steps {
    asked: Vec<(Uuid, bool)>,
    answer: Answer,
}

struct Host(Rc<RefCell<Steps>>);

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
    fn step_text_changes(&self, page: Uuid, forward: bool) -> Option<ChangesAnswer> {
        let mut steps = self.0.borrow_mut();
        steps.asked.push((page, forward));
        let (sender, answer) = std::sync::mpsc::channel();
        match steps.answer {
            Answer::NotUp => return None,
            Answer::Gone => drop(sender),
            Answer::Changes(changes) => sender.send(changes).unwrap(),
        }
        Some(answer)
    }
}

fn fixture() -> (Fixture, Rc<RefCell<Steps>>) {
    let mut f = Fixture::new(1);
    let steps = Rc::new(RefCell::new(Steps {
        asked: Vec::new(),
        answer: Answer::NotUp,
    }));
    f.core.set_html_host(Box::new(Host(Rc::clone(&steps))));
    (f, steps)
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

fn step(f: &mut Fixture, target: &str, edit: impl FnOnce(&mut Args)) -> Response {
    f.session(SessionCmd::OverlayChanges, Some(target), args(edit))
}

fn to(direction: &str) -> impl FnOnce(&mut Args) {
    move |a: &mut Args| a.to = Some(direction.to_owned())
}

fn changes(count: usize, current: usize) -> TextChanges {
    TextChanges { count, current }
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

/// Upstream's `aChangesReportIsKeptClearedAndEmitsNothing` and `aTextPageNodeReportsItsChangesOnlyOnceReported`.
#[test]
fn a_changes_report_is_kept_cleared_read_back_and_emits_nothing() {
    let (mut f, _) = fixture();
    let id = f.sessions[0];
    let page_id = open_text(&mut f, id, "a.diff", None);
    f.core.commit();
    assert!(text(&mut f, id).get("changes").is_none(), "omitted until reported");
    f.core.set_text_changes_report(page_id, Some(changes(11, 2)));
    assert_eq!(page(&f, id, None).text_changes, Some(changes(11, 2)));
    assert_eq!(text(&mut f, id)["changes"], json!({"count": 11, "current": 2}));
    f.core.set_text_changes_report(page_id, None);
    assert_eq!(page(&f, id, None).text_changes, None);
    assert!(text(&mut f, id).get("changes").is_none());
    assert!(f.core.commit().tree_changed.is_empty());
}

/// Upstream's `aChangesStepAddressesOnlyALocalDiffPage` and `testAChangesStepNeedsALoadedDiffPage`: an empty slot, a
/// plain text page, a Markdown page, a log page of a patch and a program are refused; a Markdown page takes no report.
#[test]
fn a_changes_step_addresses_only_a_local_diff_page() {
    let (mut f, steps) = fixture();
    let id = f.sessions[0];
    let target = wire(id);
    assert_eq!(step(&mut f, &target, to("next")), failure("no overlay"));
    open_text(&mut f, id, "a.txt", None);
    assert_eq!(step(&mut f, &target, to("next")), failure("no diff page"));
    assert!(f.store_mut().unwrap().close_overlay(id));

    assert!(
        f.session(
            SessionCmd::OverlayOpen,
            Some(&target),
            args(|a| a.markdown_file = Some(path("a.md")))
        )
        .ok
    );
    assert_eq!(step(&mut f, &target, to("next")), failure("no diff page"));
    let markdown = page(&f, id, None).id;
    f.core.set_text_changes_report(markdown, Some(changes(1, 0)));
    assert_eq!(page(&f, id, None).text_changes, None);
    assert!(f.store_mut().unwrap().close_overlay(id));

    let log = TextLog { tail: 10, follow: true };
    let log_page = HtmlOverlay {
        text_log: Some(log),
        log_state: Some(TextLogState::new(log)),
        ..HtmlOverlay::new(
            HtmlSource::Text {
                path: path("a.patch"),
                line: None,
            },
            HtmlPageFlags::default(),
        )
    };
    f.store_mut()
        .unwrap()
        .open_html_overlay(id, None, log_page, None, None)
        .unwrap();
    assert_eq!(step(&mut f, &target, to("next")), failure("no diff page"));
    assert!(f.store_mut().unwrap().close_overlay(id));

    assert!(f.store_mut().unwrap().open_overlay(
        id,
        OverlayProgram {
            command: "less".to_owned(),
            ..OverlayProgram::default()
        }
    ));
    assert_eq!(
        step(&mut f, &target, to("next")),
        failure("the overlay is not an html page")
    );
    assert!(steps.borrow().asked.is_empty(), "no refused step reaches a page");
    assert_eq!(TextChangesFailure::NotDiff.message(), "no diff page");
}

/// Upstream's `testAChangesStepNeedsALoadedDiffPage` and `overlayChangesRoutesItsDirectionAndPane`: a step asks the
/// page in the addressed slot and answers `N of M`; with no renderer or no answer the page is not loaded, and a page
/// with no change has none to step to.
#[test]
fn a_changes_step_needs_a_loaded_page_and_answers_where_it_went() {
    let (mut f, steps) = fixture();
    let id = f.sessions[0];
    let page_id = open_text(&mut f, id, "a.patch", Some("left"));
    let target = wire(id);
    let on_left = |direction: &'static str| {
        move |a: &mut Args| {
            a.to = Some(direction.to_owned());
            a.pane = Some("left".to_owned());
        }
    };
    assert_eq!(step(&mut f, &target, on_left("prev")), failure("page not loaded"));
    steps.borrow_mut().answer = Answer::Gone;
    assert_eq!(step(&mut f, &target, on_left("next")), failure("page not loaded"));
    steps.borrow_mut().answer = Answer::Changes(changes(0, 0));
    assert_eq!(step(&mut f, &target, on_left("next")), failure("no changes"));
    steps.borrow_mut().answer = Answer::Changes(changes(7, 3));
    assert_eq!(
        step(&mut f, &target, on_left("next")),
        Response::with_result(json!({"id": target, "text": "3 of 7"}))
    );
    assert_eq!(
        steps.borrow().asked,
        [(page_id, false), (page_id, true), (page_id, true), (page_id, true)]
    );
}

/// Upstream's `overlayChangesRejectsBadInputBeforeCallingActions`: the step, then the pane, before the session
/// resolves.
#[test]
fn changes_refuse_bad_input_before_resolving_a_target() {
    let (mut f, steps) = fixture();
    for (edit, error) in [
        (args(|_| {}), "session.overlay.changes requires to next or prev"),
        (args(to("close")), "invalid overlay changes direction: close"),
        (
            args(|a| {
                a.to = Some("next".to_owned());
                a.pane = Some("middle".to_owned());
            }),
            "session.overlay: --pane must be left or right",
        ),
        (args(to("next")), "no such session: missing"),
    ] {
        assert_eq!(
            f.session(SessionCmd::OverlayChanges, Some("missing"), edit),
            failure(error)
        );
    }
    assert!(steps.borrow().asked.is_empty());
}

/// A slot the placement rule hands to the presenter is the presenter's to step.
#[test]
fn a_presented_slot_is_stepped_where_it_is_shown() {
    let mut origin = Origin::build(false, &SUPPORTED_KINDS, false);
    assert_eq!(
        origin.overlay(SessionCmd::OverlayChanges, to("next")),
        failure("remote page: step it on the Mac that shows it")
    );
}
