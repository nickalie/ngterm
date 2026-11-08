//! Text page search (upstream's `HtmlOverlayTests` search cases, `ControlDispatcherOverlayTests.overlaySearch*`,
//! `ControlServerTextPageTests` search cases and `AppActionsPageSearchTests`): `session.overlay.search` sets, steps
//! and closes a text page's query, the page's own field and reports land through the core, and Find goes to what is on
//! top.

use std::cell::RefCell;
use std::rc::Rc;

use ngterm_host::fakes::{FakeSurface, FakeSurfaceProbe};
use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Args, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::remote_page::Origin;
use super::{Fixture, args, failure, wire};
use crate::app_actions::{ActionEffect, ActionOrigin, FrontendAction};
use crate::html::text_page::TextSearch;
use crate::html::{
    HtmlHost, HtmlNavigation, HtmlOverlay, HtmlPageFlags, HtmlSource, SearchAnswer, SearchCount, SearchStep,
};
use crate::keymap::BuiltinAction;
use crate::model::{OverlayPane, OverlayProgram, TextSearchFailure};
use crate::remote::hub::SUPPORTED_KINDS;

/// How the fake page answers a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    /// Its renderer is not up: no channel.
    NotUp,
    /// The channel closes unanswered.
    Gone,
    Count(SearchCount),
}

struct Steps {
    asked: Vec<(Uuid, SearchStep)>,
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
    fn step_text_search(&self, page: Uuid, step: SearchStep) -> Option<SearchAnswer> {
        let mut steps = self.0.borrow_mut();
        steps.asked.push((page, step));
        let (sender, answer) = std::sync::mpsc::channel();
        match steps.answer {
            Answer::NotUp => return None,
            Answer::Gone => drop(sender),
            Answer::Count(count) => sender.send(count).unwrap(),
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

fn open_text(f: &mut Fixture, id: Uuid, pane: Option<&str>) -> Uuid {
    let opened = f.session(
        SessionCmd::OverlayOpen,
        Some(&wire(id)),
        args(|a| {
            a.text_file = Some(path("a.txt"));
            a.pane = pane.map(str::to_owned);
        }),
    );
    assert!(opened.ok, "{opened:?}");
    page(f, id, pane.map(|_| OverlayPane::Left)).id
}

fn page(f: &Fixture, id: Uuid, pane: Option<OverlayPane>) -> HtmlOverlay {
    f.store().html_overlay(id, pane).unwrap().clone()
}

fn search(f: &mut Fixture, target: &str, edit: impl FnOnce(&mut Args)) -> Response {
    f.session(SessionCmd::OverlaySearch, Some(target), args(edit))
}

fn query(text: &str) -> impl FnOnce(&mut Args) {
    move |a: &mut Args| a.text = Some(text.to_owned())
}

fn to(step: &str) -> impl FnOnce(&mut Args) {
    move |a: &mut Args| a.to = Some(step.to_owned())
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

fn count(matches: usize, current: usize) -> SearchCount {
    SearchCount { matches, current }
}

/// Upstream's `aQuerySetOrClosedBumpsTheRevisionAndEmitsOnlyOnAChange`.
#[test]
fn a_query_set_or_closed_moves_the_revision_and_emits_only_on_a_change() {
    let (mut f, _) = fixture();
    let id = f.sessions[0];
    let page_id = open_text(&mut f, id, None);
    f.core.commit();
    assert_eq!(f.core.library_mut().set_text_search(page_id, "error"), Some(1));
    assert_eq!(f.core.library_mut().set_text_search(page_id, "error"), None);
    let set = page(&f, id, None);
    assert_eq!(
        (set.text_search, set.search_revision),
        (Some(TextSearch::new("error")), 1)
    );
    assert_eq!(f.core.commit().tree_changed.len(), 1);
    assert_eq!(f.core.library_mut().set_text_search(page_id, ""), Some(2));
    assert_eq!(f.core.library_mut().set_text_search(page_id, ""), None);
    let closed = page(&f, id, None);
    assert_eq!((closed.text_search, closed.search_revision), (None, 2));
    assert_eq!(f.core.commit().tree_changed.len(), 1);
}

/// Upstream's `aReportIsKeptOnlyForTheCurrentRevisionAndEmitsNothing`, `aClosedSearchDropsALateReport` and
/// `dfbf1f3`'s query check: a report lands only under its own revision and query.
#[test]
fn a_report_lands_only_for_the_current_revision_and_query_and_emits_nothing() {
    let (mut f, _) = fixture();
    let id = f.sessions[0];
    let page_id = open_text(&mut f, id, None);
    f.core.library_mut().set_text_search(page_id, "a");
    f.core.library_mut().set_text_search(page_id, "ab");
    f.core.commit();
    f.core.set_text_search_report(page_id, 1, "a", count(9, 4));
    f.core.set_text_search_report(page_id, 2, "a", count(9, 4));
    assert_eq!(page(&f, id, None).text_search, Some(TextSearch::new("ab")));
    f.core.set_text_search_report(page_id, 2, "ab", count(3, 1));
    assert_eq!(
        page(&f, id, None).text_search,
        Some(TextSearch {
            query: "ab".to_owned(),
            matches: 3,
            current: 1
        })
    );
    assert!(f.core.commit().tree_changed.is_empty());
    // a closed search drops a late report
    f.core.library_mut().set_text_search(page_id, "");
    f.core.set_text_search_report(page_id, 3, "", count(1, 1));
    assert_eq!(page(&f, id, None).text_search, None);
}

/// The page's own field writes the same model: its revision, and none for an unchanged query.
#[test]
fn the_pages_own_field_writes_the_model() {
    let (mut f, _) = fixture();
    let id = f.sessions[0];
    let page_id = open_text(&mut f, id, None);
    f.core.commit();
    assert_eq!(f.core.set_text_search_from_page(page_id, "b"), Some(1));
    assert_eq!(f.core.set_text_search_from_page(page_id, "b"), None);
    assert_eq!(f.core.commit().tree_changed.len(), 1);
    assert_eq!(
        text(&mut f, id)["search"],
        json!({"query": "b", "matches": 0, "current": 0})
    );
}

/// Upstream's `aSearchAddressesOnlyALocalTextPage` and `aTextPageNodeReportsItsSearch`: a markdown page, a replica
/// and a program are refused, and a query never lands on a page that is not a text page.
#[test]
fn a_search_addresses_only_a_local_text_page() {
    let (mut f, _) = fixture();
    let id = f.sessions[0];
    let target = wire(id);
    assert_eq!(search(&mut f, &target, query("a")), failure("no overlay"));
    assert!(
        f.session(
            SessionCmd::OverlayOpen,
            Some(&target),
            args(|a| a.markdown_file = Some(path("a.md")))
        )
        .ok
    );
    assert_eq!(search(&mut f, &target, query("a")), failure("no text page"));
    let markdown = page(&f, id, None).id;
    assert_eq!(f.core.library_mut().set_text_search(markdown, "a"), None);
    assert_eq!(page(&f, id, None).text_search, None);
    assert!(f.store_mut().unwrap().close_overlay(id));

    // a text page shown for another machine is static there: no control search reaches it (#891)
    for content in [
        crate::html::ReplicaContent::Html,
        crate::html::ReplicaContent::Text { line: None },
    ] {
        let replica = HtmlOverlay {
            replica_of: Some(Uuid::new_v4()),
            ..HtmlOverlay::new(
                HtmlSource::Replica {
                    path: "/tmp/a.txt".to_owned(),
                    content,
                },
                HtmlPageFlags::default(),
            )
        };
        f.store_mut()
            .unwrap()
            .open_html_overlay(id, None, replica, None, None)
            .unwrap();
        assert_eq!(search(&mut f, &target, query("a")), failure("no text page"));
        assert!(f.store_mut().unwrap().close_overlay(id));
    }

    assert!(f.store_mut().unwrap().open_overlay(
        id,
        OverlayProgram {
            command: "less".to_owned(),
            ..OverlayProgram::default()
        }
    ));
    assert_eq!(
        search(&mut f, &target, query("a")),
        failure("the overlay is not an html page")
    );
    assert_eq!(TextSearchFailure::NotText.message(), "no text page");
}

/// Upstream's `testSearchSetsAndClosesTheQueryOnTheModelAndItsReadBack`: a query and a close answer the session's id,
/// and the tree reads the search back, omitted with none.
#[test]
fn search_sets_and_closes_the_query_and_the_tree_reads_it_back() {
    let (mut f, steps) = fixture();
    let id = f.sessions[0];
    open_text(&mut f, id, None);
    f.core.commit();
    assert!(text(&mut f, id).get("search").is_none());
    let answered = Response::with_result(json!({"id": wire(id)}));
    assert_eq!(search(&mut f, &wire(id), query("two")), answered);
    assert_eq!(f.core.commit().tree_changed.len(), 1);
    assert_eq!(
        text(&mut f, id)["search"],
        json!({"query": "two", "matches": 0, "current": 0})
    );
    assert_eq!(search(&mut f, &wire(id), to("close")), answered);
    assert!(text(&mut f, id).get("search").is_none());
    assert!(
        steps.borrow().asked.is_empty(),
        "a query and a close go through the model alone"
    );
}

/// Upstream's `testAStepNeedsAQueryAndALoadedPage` and `overlaySearchRoutesItsRequestAndPane`: a step asks the page
/// with the model's query and revision and answers `N of M`; with no query, no renderer or no answer it is refused.
#[test]
fn a_step_needs_a_query_and_a_loaded_page_and_answers_its_count() {
    let (mut f, steps) = fixture();
    let id = f.sessions[0];
    let page_id = open_text(&mut f, id, Some("left"));
    let target = wire(id);
    let on_left = |step: &'static str| {
        move |a: &mut Args| {
            a.to = Some(step.to_owned());
            a.pane = Some("left".to_owned());
        }
    };
    assert_eq!(
        search(&mut f, &target, on_left("next")),
        failure("no search query: set one first")
    );
    assert!(steps.borrow().asked.is_empty());
    assert!(
        search(&mut f, &target, |a| {
            a.text = Some("one".to_owned());
            a.pane = Some("left".to_owned());
        })
        .ok
    );
    assert_eq!(search(&mut f, &target, on_left("prev")), failure("page not loaded"));
    steps.borrow_mut().answer = Answer::Gone;
    assert_eq!(search(&mut f, &target, on_left("prev")), failure("page not loaded"));
    steps.borrow_mut().answer = Answer::Count(count(2, 1));
    assert_eq!(
        search(&mut f, &target, on_left("next")),
        Response::with_result(json!({"id": target, "text": "1 of 2"}))
    );
    let asked = &steps.borrow().asked;
    assert_eq!(asked.len(), 3);
    assert_eq!(
        asked[2],
        (
            page_id,
            SearchStep {
                forward: true,
                query: "one".to_owned(),
                revision: 1
            }
        )
    );
    assert!(!asked[0].1.forward, "prev steps back");
}

/// Upstream's `overlaySearchRejectsBadInputBeforeCallingActions`: the request, then the pane, before the session
/// resolves.
#[test]
fn search_refuses_bad_input_before_resolving_a_target() {
    let (mut f, steps) = fixture();
    for (edit, error) in [
        (
            args(|_| {}),
            "session.overlay.search needs text, --next, --prev or --close",
        ),
        (
            args(|a| {
                a.text = Some("a".to_owned());
                a.to = Some("next".to_owned());
            }),
            "session.overlay.search: text cannot be combined with --next, --prev or --close",
        ),
        (args(query("")), "session.overlay.search: text must not be empty"),
        (args(to("up")), "invalid overlay search direction: up"),
        (
            args(|a| {
                a.text = Some("a".to_owned());
                a.pane = Some("middle".to_owned());
            }),
            "session.overlay: --pane must be left or right",
        ),
        (args(query("a")), "no such session: missing"),
    ] {
        assert_eq!(
            f.session(SessionCmd::OverlaySearch, Some("missing"), edit),
            failure(error)
        );
    }
    assert!(steps.borrow().asked.is_empty());
}

/// Upstream's `testSearchRefusesAnEmptySlotAndAPresentedPage`: a slot the placement rule hands to the presenter is
/// the presenter's to search.
#[test]
fn a_presented_slot_is_searched_where_it_is_shown() {
    let mut origin = Origin::build(false, &SUPPORTED_KINDS, false);
    assert_eq!(
        origin.overlay(SessionCmd::OverlaySearch, query("a")),
        failure("remote page: search it on the Mac that shows it")
    );
}

// MARK: - Find goes to what is on top

fn find(f: &mut Fixture, quick: bool) -> ActionEffect {
    let origin = ActionOrigin {
        window: f.window,
        quick,
    };
    let effect = f.core.perform_builtin(origin, BuiltinAction::ToggleSearch);
    let _ = f.core.commit();
    effect
}

/// Gives session `id` a terminal in each pane; the probes read the binding actions they run.
fn terminals(f: &mut Fixture, id: Uuid) -> (FakeSurfaceProbe, FakeSurfaceProbe) {
    let (left, right) = (FakeSurface::new("L"), FakeSurface::new("R"));
    let probes = (left.probe(), right.probe());
    let session = f.store_mut().unwrap().session_mut(id).unwrap();
    session.surface.replace(Box::new(left));
    session.split_surface.replace(Box::new(right));
    probes
}

/// A floating text page shown for another machine takes Find into its own field too (upstream's `kind == .text`).
#[test]
fn a_text_replica_takes_find_into_its_own_field() {
    let (mut f, _) = fixture();
    let id = f.selected().unwrap();
    terminals(&mut f, id);
    let replica = HtmlOverlay {
        replica_of: Some(Uuid::new_v4()),
        ..HtmlOverlay::new(
            HtmlSource::Replica {
                path: "/tmp/a.txt".to_owned(),
                content: crate::html::ReplicaContent::Text { line: None },
            },
            HtmlPageFlags::default(),
        )
    };
    let shown = replica.id;
    let store = f.store_mut().unwrap();
    store.open_html_overlay(id, None, replica, None, None).unwrap();
    assert!(store.resize_overlay_with_points(id, Some(60), crate::model::OverlayPointSize::default()));
    assert_eq!(
        find(&mut f, false),
        ActionEffect::Frontend(FrontendAction::FocusPageSearch {
            window: f.window,
            page: shown
        })
    );
    assert!(f.store_mut().unwrap().close_overlay(id));
}

/// Upstream's `testEveryShownCoverOverTheFocusedPaneTakesCommandF`: a floating text page takes Find into its own field,
/// a floating markdown page or program takes it and does nothing; the terminal behind starts no search.
#[test]
fn every_shown_cover_over_the_focused_pane_takes_find() {
    let (mut f, _) = fixture();
    let id = f.selected().unwrap();
    let (left, _) = terminals(&mut f, id);
    let target = wire(id);
    assert!(
        f.session(
            SessionCmd::OverlayOpen,
            Some(&target),
            args(|a| {
                a.text_file = Some(path("a.txt"));
                a.size_percent = Some(60);
            })
        )
        .ok
    );
    let text_page = page(&f, id, None).id;
    assert_eq!(
        find(&mut f, false),
        ActionEffect::Frontend(FrontendAction::FocusPageSearch {
            window: f.window,
            page: text_page
        })
    );
    assert!(f.store_mut().unwrap().close_overlay(id));
    assert!(
        f.session(
            SessionCmd::OverlayOpen,
            Some(&target),
            args(|a| {
                a.markdown_file = Some(path("a.md"));
                a.size_percent = Some(60);
            })
        )
        .ok
    );
    assert_eq!(find(&mut f, false), ActionEffect::Done);
    assert!(f.store_mut().unwrap().close_overlay(id));
    assert!(f.store_mut().unwrap().open_overlay(
        id,
        OverlayProgram {
            command: "less".to_owned(),
            size_percent: Some(60),
            ..OverlayProgram::default()
        }
    ));
    assert_eq!(find(&mut f, false), ActionEffect::Done);
    assert!(left.actions().is_empty(), "the terminal behind keeps its bar closed");
    assert!(f.store_mut().unwrap().close_overlay(id));
    // the quick terminal holding the keys takes it too
    assert_eq!(find(&mut f, true), ActionEffect::Done);
    assert!(left.actions().is_empty());
    // with nothing on top the terminal keeps it
    assert_eq!(find(&mut f, false), ActionEffect::Done);
    assert_eq!(left.actions(), ["start_search"]);
}

/// Upstream's `testAHiddenPageOrAPageOverTheOtherPaneLeavesCommandFToTheTerminal`.
#[test]
fn a_hidden_page_or_a_page_over_the_other_pane_leaves_find_to_the_terminal() {
    let (mut f, _) = fixture();
    let id = f.selected().unwrap();
    let (left, right) = terminals(&mut f, id);
    assert!(
        f.session(
            SessionCmd::OverlayOpen,
            Some(&wire(id)),
            args(|a| {
                a.markdown_file = Some(path("a.md"));
                a.size_percent = Some(60);
            })
        )
        .ok
    );
    f.store_mut().unwrap().session_mut(id).unwrap().overlay.hidden = true;
    assert_eq!(find(&mut f, false), ActionEffect::Done);
    assert_eq!(left.actions(), ["start_search"], "a hidden page is not on top");
    assert!(f.store_mut().unwrap().close_overlay(id));

    let store = f.store_mut().unwrap();
    store.set_split_visibility(id, true, None);
    assert_eq!(
        store.open_pane_overlay(
            id,
            OverlayPane::Left,
            OverlayProgram {
                command: "htop".to_owned(),
                ..OverlayProgram::default()
            }
        ),
        Ok(())
    );
    let session = store.session_mut(id).unwrap();
    session.search.active = false;
    session.split.focused = true;
    assert_eq!(find(&mut f, false), ActionEffect::Done);
    assert_eq!(
        right.actions(),
        ["start_search"],
        "the focused right pane has nothing over it"
    );
    let session = f.store_mut().unwrap().session_mut(id).unwrap();
    session.search.active = false;
    session.split.focused = false;
    assert_eq!(find(&mut f, false), ActionEffect::Done);
    assert_eq!(
        left.actions(),
        ["start_search"],
        "the focused left pane is under its own overlay"
    );
}
