//! Terminal I/O over fake surfaces: `session.type`, `text`, `copy`, `paste`, `selectall`, `search`, `surface.*`.

use ngterm_host::fakes::{FakeSurface, FakeSurfaceProbe};
use ngterm_proto::command::{SessionCmd, SurfaceCmd};
use ngterm_proto::{Command, Request, Response};
use serde_json::json;
use uuid::Uuid;

use super::{Fixture, UNKNOWN, args, failure, wire};
use crate::Dispatched;
use crate::model::SearchReport;

const TOKEN: &str = "TOKEN-L";

/// One case's change to the request args.
type ArgsEdit = Box<dyn Fn(&mut ngterm_proto::Args)>;

/// One window with two sessions, the second selected; the first one's main pane holds a fake terminal.
fn fixture_with_terminal() -> (Fixture, Uuid, FakeSurfaceProbe) {
    let mut fixture = Fixture::new(1);
    let id = fixture.sessions[0];
    let probe = install(&mut fixture, id, FakeSurface::new(TOKEN));
    (fixture, id, probe)
}

fn install(fixture: &mut Fixture, id: Uuid, surface: FakeSurface) -> FakeSurfaceProbe {
    let probe = surface.probe();
    let session = fixture.store_mut().and_then(|store| store.session_mut(id)).unwrap();
    session.surface.replace(Box::new(surface));
    probe
}

fn result(response: &Response) -> serde_json::Value {
    assert!(response.ok, "{response:?}");
    response.result.clone().unwrap_or_default()
}

fn request(cmd: Command, target: Option<&str>, edit: impl FnOnce(&mut ngterm_proto::Args)) -> Request {
    Request {
        cmd,
        target: target.map(str::to_owned),
        args: Some(args(edit)),
    }
}

fn resume(core: &mut crate::Core, dispatched: Dispatched) -> Dispatched {
    match dispatched {
        Dispatched::Pending { resume, .. } => resume(core),
        done @ Dispatched::Done(_) => done,
    }
}

fn answer(dispatched: Dispatched) -> Response {
    match dispatched {
        Dispatched::Done(response) => response,
        Dispatched::Pending { .. } => panic!("still pending"),
    }
}

// MARK: - session.type

#[test]
fn type_queues_the_text_on_the_main_pane_and_names_it() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    let target = wire(id);
    let response = fixture.session(SessionCmd::Type, Some(&target), args(|a| a.text = Some("ls\n".into())));
    assert_eq!(result(&response), json!({"id": target, "pane": "left"}));
    assert_eq!(probe.typed(), ["ls\n"]);
    // typing into a realized pane never moves the selection, `--select` or not
    let response = fixture.session(
        SessionCmd::Type,
        Some(&target),
        args(|a| {
            a.text = Some("x".into());
            a.select = Some(true);
        }),
    );
    assert!(response.ok);
    assert_ne!(fixture.selected(), Some(id));
}

#[test]
fn type_refuses_as_upstream() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    let target = wire(id);
    let cases: Vec<(ArgsEdit, &str)> = vec![
        (Box::new(|_| {}), "session.type requires text"),
        (
            Box::new(|a| a.text = Some("a\0b".into())),
            "text must not contain a NUL byte",
        ),
        (
            Box::new(|a| {
                a.text = Some("x".into());
                a.pane = Some("middle".into());
            }),
            "invalid pane: middle",
        ),
        (
            Box::new(|a| {
                a.text = Some("x".into());
                a.pane_id = Some(UNKNOWN.into());
            }),
            "unknown pane id: zzz",
        ),
        (
            Box::new(|a| {
                a.text = Some("x".into());
                a.pane = Some("right".into());
            }),
            "session has no split pane",
        ),
        (
            Box::new(|a| {
                a.text = Some("x".into());
                a.pane = Some("scratch".into());
            }),
            "session has no scratch terminal",
        ),
    ];
    for (edit, error) in cases {
        assert_eq!(
            fixture.session(SessionCmd::Type, Some(&target), args(edit)),
            failure(error)
        );
    }
    assert_eq!(
        fixture.session(SessionCmd::Type, Some(UNKNOWN), args(|a| a.text = Some("x".into()))),
        failure("no such session: zzz")
    );
    assert!(probe.typed().is_empty());
}

#[test]
fn type_follows_a_pane_token_and_falls_back_to_the_named_role() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    let target = wire(id);
    let response = fixture.session(
        SessionCmd::Type,
        Some(&target),
        args(|a| {
            a.text = Some("by token".into());
            a.pane_id = Some(TOKEN.into());
            a.pane = Some("right".into());
        }),
    );
    assert_eq!(result(&response)["pane"], "left");
    let response = fixture.session(
        SessionCmd::Type,
        Some(&target),
        args(|a| {
            a.text = Some("fallback".into());
            a.pane_id = Some("gone".into());
            a.pane = Some("primary".into());
        }),
    );
    assert_eq!(result(&response)["pane"], "left");
    assert_eq!(probe.typed(), ["by token", "fallback"]);
}

#[test]
fn type_waits_for_a_main_pane_coming_up_and_selects_when_asked() {
    let mut fixture = Fixture::new(1);
    let id = fixture.sessions[0];
    let probe = install(&mut fixture, id, FakeSurface::new(TOKEN).with_realized(false));
    let request = request(Command::Session(SessionCmd::Type), Some(&wire(id)), |a| {
        a.text = Some("late\n".into());
        a.select = Some(true);
    });
    let pending = fixture.core.begin(&request);
    assert!(matches!(pending, Dispatched::Pending { .. }));
    assert_eq!(fixture.selected(), Some(id));
    let pending = resume(&mut fixture.core, pending);
    assert!(matches!(pending, Dispatched::Pending { .. }));
    probe.set_realized(true);
    let response = answer(resume(&mut fixture.core, pending));
    assert_eq!(result(&response), json!({"id": wire(id), "pane": "left"}));
    assert_eq!(probe.typed(), ["late\n"]);
}

#[test]
fn type_into_a_terminal_that_never_comes_up_is_refused_without_selecting() {
    let mut fixture = Fixture::new(1);
    let id = fixture.sessions[0];
    let probe = install(&mut fixture, id, FakeSurface::new(TOKEN).with_realized(false));
    let response = fixture.session(SessionCmd::Type, Some(&wire(id)), args(|a| a.text = Some("x".into())));
    assert_eq!(response, failure("session not realized"));
    assert_ne!(fixture.selected(), Some(id));
    assert!(probe.typed().is_empty());
    // a session without any terminal waits the same way
    let other = fixture.sessions[1];
    let response = fixture.session(
        SessionCmd::Type,
        Some(&wire(other)),
        args(|a| a.text = Some("x".into())),
    );
    assert_eq!(response, failure("session not realized"));
}

#[test]
fn typed_input_clears_a_blocked_status_as_the_keyboard_does() {
    let (mut fixture, id, _probe) = fixture_with_terminal();
    let target = wire(id);
    let blocked = fixture.session(
        SessionCmd::Status,
        Some(&target),
        args(|a| a.status = Some("blocked".into())),
    );
    assert!(blocked.ok);
    let response = fixture.session(SessionCmd::Type, Some(&target), args(|a| a.text = Some("yes\n".into())));
    assert!(response.ok);
    let status = fixture
        .store()
        .session(id)
        .map(|session| session.agent_indicator.status);
    assert_eq!(status, Some(crate::model::AgentStatus::Idle));
}

#[test]
fn a_keystroke_in_the_owning_pane_clears_the_status_as_upstream() {
    use crate::model::{AgentStatus, StatusKeystroke};

    let (mut fixture, id, _probe) = fixture_with_terminal();
    let target = wire(id);
    let status = |fixture: &Fixture| {
        fixture
            .store()
            .session(id)
            .map(|session| session.agent_indicator.status)
    };
    let set = |fixture: &mut Fixture, status: &str| {
        let response = fixture.session(
            SessionCmd::Status,
            Some(&target),
            args(|a| a.status = Some(status.into())),
        );
        assert!(response.ok, "{response:?}");
    };

    // active survives plain typing and a submit, an interrupt drops it
    set(&mut fixture, "active");
    assert!(
        !fixture
            .core
            .clear_status_on_keystroke(id, TOKEN, StatusKeystroke::Other)
    );
    assert!(
        !fixture
            .core
            .clear_status_on_keystroke(id, TOKEN, StatusKeystroke::Submit)
    );
    assert_eq!(status(&fixture), Some(AgentStatus::Active));
    assert!(
        fixture
            .core
            .clear_status_on_keystroke(id, TOKEN, StatusKeystroke::Interrupt)
    );
    assert_eq!(status(&fixture), Some(AgentStatus::Idle));

    // blocked clears on the first key under the default Status reset
    set(&mut fixture, "blocked");
    // a terminal that is not this session's pane clears nothing
    assert!(
        !fixture
            .core
            .clear_status_on_keystroke(id, "other", StatusKeystroke::Other)
    );
    assert!(
        !fixture
            .core
            .clear_status_on_keystroke(Uuid::new_v4(), TOKEN, StatusKeystroke::Other)
    );
    assert_eq!(status(&fixture), Some(AgentStatus::Blocked));
    assert!(
        fixture
            .core
            .clear_status_on_keystroke(id, TOKEN, StatusKeystroke::Other)
    );
    assert_eq!(status(&fixture), Some(AgentStatus::Idle));
}

// MARK: - session.text

#[test]
fn text_reads_the_screen_the_scrollback_or_the_last_content_lines() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    let target = wire(id);
    let mut lines: Vec<String> = (0..30).map(|n| format!("line {n}")).collect();
    lines.extend(["$ ".to_owned(), String::new(), "   ".to_owned()]);
    probe.set_lines(lines);
    let screen = result(&fixture.session(SessionCmd::Text, Some(&target), args(|_| {})));
    assert_eq!(screen["pane"], "left");
    assert!(screen["text"].as_str().unwrap().starts_with("line 9\n"));
    let all = result(&fixture.session(SessionCmd::Text, Some(&target), args(|a| a.all = Some(true))));
    assert!(all["text"].as_str().unwrap().starts_with("line 0\n"));
    let last = result(&fixture.session(SessionCmd::Text, Some(&target), args(|a| a.lines = Some(2))));
    assert_eq!(last, json!({"text": "line 29\n$ ", "pane": "left"}));
    let by_token = fixture.session(
        SessionCmd::Text,
        Some(&target),
        args(|a| a.pane_id = Some(TOKEN.into())),
    );
    assert_eq!(result(&by_token)["pane"], "left");
}

#[test]
fn text_refuses_as_upstream() {
    let (mut fixture, id, _probe) = fixture_with_terminal();
    let target = wire(id);
    let cases: Vec<(ArgsEdit, &str)> = vec![
        (Box::new(|a| a.lines = Some(0)), "--lines must be greater than 0"),
        (Box::new(|a| a.lines = Some(-1)), "--lines must be greater than 0"),
        (
            Box::new(|a| {
                a.all = Some(true);
                a.lines = Some(3);
            }),
            "use either --all or --lines, not both",
        ),
        (Box::new(|a| a.pane = Some("middle".into())), "invalid pane: middle"),
        (Box::new(|a| a.pane_id = Some(UNKNOWN.into())), "unknown pane id: zzz"),
        (Box::new(|a| a.pane = Some("right".into())), "session has no split pane"),
        (
            Box::new(|a| a.pane = Some("scratch".into())),
            "session has no scratch terminal",
        ),
    ];
    for (edit, error) in cases {
        assert_eq!(
            fixture.session(SessionCmd::Text, Some(&target), args(edit)),
            failure(error)
        );
    }
    assert_eq!(
        fixture.session(SessionCmd::Text, Some(UNKNOWN), args(|_| {})),
        failure("no such session: zzz")
    );
    // a session whose terminal is not up yet, or has none
    let other = wire(fixture.sessions[1]);
    assert_eq!(
        fixture.session(SessionCmd::Text, Some(&other), args(|_| {})),
        failure("session not realized")
    );
}

// MARK: - selection

#[test]
fn selectall_then_copy_reads_the_selection_back_and_paste_runs_on_the_pane() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    let target = wire(id);
    probe.set_lines(["one", "two"]);
    assert_eq!(
        fixture.session(SessionCmd::Copy, Some(&target), args(|_| {})),
        failure("no selection")
    );
    let response = fixture.session(SessionCmd::SelectAll, Some(&target), args(|_| {}));
    assert_eq!(result(&response), json!({"id": target}));
    let response = fixture.session(SessionCmd::Copy, Some(&target), args(|_| {}));
    assert_eq!(result(&response), json!({"text": "one\ntwo"}));
    let response = fixture.session(
        SessionCmd::Paste,
        Some(&target),
        args(|a| a.pane = Some("primary".into())),
    );
    assert_eq!(result(&response), json!({"id": target}));
    assert_eq!(probe.actions(), ["select_all", "paste_from_clipboard"]);
}

#[test]
fn selection_commands_refuse_as_upstream() {
    let (mut fixture, id, _probe) = fixture_with_terminal();
    let target = wire(id);
    for (pane, error) in [
        ("middle", "--pane must be left, right, or scratch"),
        ("right", "session has no split pane"),
        ("split", "session has no split pane"),
        ("scratch", "session has no scratch terminal"),
    ] {
        assert_eq!(
            fixture.session(SessionCmd::Paste, Some(&target), args(|a| a.pane = Some(pane.into()))),
            failure(error)
        );
    }
    for cmd in [SessionCmd::SelectAll, SessionCmd::Copy, SessionCmd::Paste] {
        assert_eq!(
            fixture.session(cmd, Some(UNKNOWN), args(|_| {})),
            failure("no such session: zzz")
        );
    }
    let other = wire(fixture.sessions[1]);
    for cmd in [SessionCmd::SelectAll, SessionCmd::Copy, SessionCmd::Paste] {
        assert_eq!(
            fixture.session(cmd, Some(&other), args(|_| {})),
            failure("session not realized")
        );
    }
}

// MARK: - session.search

#[test]
fn search_opens_queries_and_answers_with_the_count_libghostty_reports() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    let target = wire(id);
    let request = request(Command::Session(SessionCmd::Search), Some(&target), |a| {
        a.text = Some("needle".into());
    });
    let pending = fixture.core.begin(&request);
    assert_eq!(fixture.selected(), Some(id));
    assert_eq!(probe.actions(), ["start_search"]);
    fixture.core.report_search(id, TOKEN, SearchReport::Started(None));
    // the needle goes out a beat later, then the count is awaited
    let pending = resume(&mut fixture.core, pending);
    assert_eq!(probe.actions(), ["start_search", "search:needle"]);
    fixture.core.report_search(id, TOKEN, SearchReport::Total(Some(2)));
    let response = answer(resume(&mut fixture.core, pending));
    assert_eq!(result(&response), json!({"text": "2 matches", "count": 2}));

    // the same needle again sends it without clearing the count, then steps the selection
    let response = fixture.send(
        Command::Session(SessionCmd::Search),
        Some(&target),
        args(|a| {
            a.text = Some("needle".into());
            a.to = Some("next".into());
        }),
    );
    assert_eq!(result(&response)["count"], 2);
    assert_eq!(
        probe.actions()[2..],
        ["search:needle".to_owned(), "navigate_search:previous".to_owned()]
    );

    let response = fixture.send(
        Command::Session(SessionCmd::Search),
        Some(&target),
        args(|a| a.to = Some("close".into())),
    );
    assert_eq!(result(&response), json!({"id": target}));
    assert_eq!(probe.actions().last().map(String::as_str), Some("end_search"));
}

#[test]
fn a_second_start_while_open_ends_the_search_on_its_pane() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    fixture
        .core
        .report_search(id, TOKEN, SearchReport::Started(Some("seed".into())));
    fixture.core.report_search(id, TOKEN, SearchReport::Started(None));
    assert_eq!(probe.actions(), ["end_search"]);
    fixture.core.report_search(id, TOKEN, SearchReport::Ended);
    // an unknown token lands nowhere
    fixture.core.report_search(id, "other", SearchReport::Started(None));
    let session = fixture.store().session(id).unwrap();
    assert!(!session.search.active);
}

#[test]
fn search_refuses_as_upstream() {
    let (mut fixture, id, _probe) = fixture_with_terminal();
    let target = wire(id);
    let search = |fixture: &mut Fixture, target: &str, to: &str| {
        fixture.send(
            Command::Session(SessionCmd::Search),
            Some(target),
            args(|a| a.to = Some(to.into())),
        )
    };
    // the target is resolved before the mode is checked
    assert_eq!(
        search(&mut fixture, UNKNOWN, "sideways"),
        failure("no such session: zzz")
    );
    assert_eq!(
        search(&mut fixture, &target, "sideways"),
        failure("session.search --to must be next|prev|close")
    );
    let other = wire(fixture.sessions[1]);
    assert_eq!(search(&mut fixture, &other, "next"), failure("session not realized"));
    let zoomed = fixture.send(
        Command::Surface(SurfaceCmd::Zoom),
        None,
        args(|a| a.mode = Some("show".into())),
    );
    assert!(zoomed.ok);
    assert_eq!(search(&mut fixture, &target, "next"), failure("terminal zoom active"));
}

/// Drives `dispatched` to its answer, calling `between` before each pending step; the number of steps it took.
fn settle(
    core: &mut crate::Core,
    mut dispatched: Dispatched,
    mut between: impl FnMut(&mut crate::Core),
) -> (Response, usize) {
    let mut steps = 0;
    loop {
        match dispatched {
            Dispatched::Done(response) => return (response, steps),
            Dispatched::Pending { resume, .. } => {
                between(core);
                steps += 1;
                dispatched = resume(core);
            }
        }
    }
}

/// Opens search on session `id` with `needle` and lands libghostty's count of `total`.
fn searched(fixture: &mut Fixture, id: Uuid, needle: &str, total: usize) {
    let request = request(Command::Session(SessionCmd::Search), Some(&wire(id)), |a| {
        a.text = Some(needle.into());
    });
    let pending = fixture.core.begin(&request);
    fixture.core.report_search(id, TOKEN, SearchReport::Started(None));
    let (response, _) = settle(&mut fixture.core, pending, |core| {
        core.report_search(id, TOKEN, SearchReport::Total(Some(total)));
    });
    assert!(response.ok, "{response:?}");
}

/// `--to` alone steps the open search and answers with the count it holds.
#[test]
fn a_step_without_a_needle_moves_the_selection_and_reads_the_count() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    searched(&mut fixture, id, "needle", 4);
    let response = fixture.send(
        Command::Session(SessionCmd::Search),
        Some(&wire(id)),
        args(|a| a.to = Some("prev".into())),
    );
    assert_eq!(result(&response), json!({"text": "4 matches", "count": 4}));
    assert_eq!(probe.actions().last().map(String::as_str), Some("navigate_search:next"));
}

/// An empty needle clears the query and answers at once: libghostty reports no count for it.
#[test]
fn an_empty_needle_clears_the_count_and_answers_with_the_session() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    searched(&mut fixture, id, "needle", 2);
    let request = request(Command::Session(SessionCmd::Search), Some(&wire(id)), |a| {
        a.text = Some(String::new());
    });
    let pending = fixture.core.begin(&request);
    let (response, steps) = settle(&mut fixture.core, pending, |_| {});
    assert_eq!(result(&response), json!({"id": wire(id)}));
    assert_eq!(steps, 1, "only the beat for the older count");
    assert_eq!(probe.actions().last().map(String::as_str), Some("search:"));
    let search = &fixture.store().session(id).unwrap().search;
    assert_eq!((search.needle.as_str(), search.total), ("", None));
}

/// Upstream's 16 probes: a count that never comes answers without one, and the counter is left out.
#[test]
fn a_count_that_never_arrives_answers_after_the_last_probe() {
    let (mut fixture, id, _probe) = fixture_with_terminal();
    let request = request(Command::Session(SessionCmd::Search), Some(&wire(id)), |a| {
        a.text = Some("needle".into());
    });
    let pending = fixture.core.begin(&request);
    let (response, steps) = settle(&mut fixture.core, pending, |_| {});
    assert!(response.ok, "{response:?}");
    assert_eq!(result(&response), json!({}));
    assert_eq!(steps, 1 + 16, "the needle's beat, then sixteen probes");
}

/// A count landing on a later probe is the one answered.
#[test]
fn a_late_count_is_awaited() {
    let (mut fixture, id, _probe) = fixture_with_terminal();
    let request = request(Command::Session(SessionCmd::Search), Some(&wire(id)), |a| {
        a.text = Some("needle".into());
    });
    let pending = fixture.core.begin(&request);
    let mut probes = 0;
    let (response, _) = settle(&mut fixture.core, pending, |core| {
        probes += 1;
        if probes == 5 {
            core.report_search(id, TOKEN, SearchReport::Started(None));
            core.report_search(id, TOKEN, SearchReport::Total(Some(0)));
        }
    });
    assert_eq!(result(&response), json!({"text": "no matches", "count": 0}));
    assert_eq!(probes, 5);
}

/// A session closed while a search waits on it ends the search with the session's absence, whatever it waited for.
#[test]
fn a_session_closed_mid_search_is_reported_gone() {
    // the wait it closes in: the terminal's coming up, the older count's beat, the count
    for (close_at, realized) in [(0, false), (0, true), (1, true)] {
        let (mut fixture, id, probe) = fixture_with_terminal();
        probe.set_realized(realized);
        let request = request(Command::Session(SessionCmd::Search), Some(&wire(id)), |a| {
            a.text = Some("needle".into());
        });
        let pending = fixture.core.begin(&request);
        let window = fixture.window;
        let mut step = 0;
        let (response, _) = settle(&mut fixture.core, pending, |core| {
            if step == close_at {
                core.library_mut().store_mut(window).unwrap().close_session(id);
            }
            step += 1;
        });
        assert_eq!(response, failure("no such session"), "closed at step {close_at}");
    }
}

/// The search bar's needle, steps and close reach nothing while no search is open, the session's search state
/// included.
#[test]
fn the_search_bar_does_nothing_without_an_open_search() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    let window = fixture.window;
    fixture.store_mut().unwrap().select_session(Some(id));
    fixture.core.search_bar_needle(window, "x");
    fixture.core.search_bar_step(window, true);
    fixture.core.search_bar_close(window);
    fixture.core.search_bar_needle(Uuid::from_u128(9), "x");
    assert!(probe.actions().is_empty());
    let search = &fixture.store().session(id).unwrap().search;
    assert_eq!(
        *search,
        crate::model::SearchState::default(),
        "the needle is kept only while a search is open"
    );
}

// MARK: - the search bar

#[test]
fn the_search_bar_drives_the_pinned_pane_and_its_end_refocuses_the_terminal() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    let window = fixture.window;
    fixture.store_mut().unwrap().select_session(Some(id));
    fixture.core.search_bar_needle(window, "ignored");
    assert!(probe.actions().is_empty(), "no search is open");
    fixture.core.report_search(id, TOKEN, SearchReport::Started(None));
    fixture.core.search_bar_needle(window, "err");
    fixture.core.report_search(id, TOKEN, SearchReport::Total(Some(3)));
    fixture.core.search_bar_step(window, true);
    fixture.core.search_bar_step(window, false);
    assert_eq!(
        probe.actions(),
        ["search:err", "navigate_search:previous", "navigate_search:next"],
        "next is visually down, libghostty's previous"
    );
    let search = &fixture.store().session(id).unwrap().search;
    assert_eq!((search.needle.as_str(), search.total), ("err", Some(3)));
    fixture.core.search_bar_needle(window, "");
    let search = &fixture.store().session(id).unwrap().search;
    assert_eq!(search.total, None, "an empty needle blanks the counter at once");
    fixture.core.search_bar_close(window);
    assert_eq!(probe.actions().last().map(String::as_str), Some("end_search"));
    fixture.core.clear_pane_focus_requests();
    fixture.core.report_search(id, TOKEN, SearchReport::Ended);
    assert!(fixture.core.pane_focus_requested(id));
}

#[test]
fn a_zoom_ends_the_open_search() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    fixture.store_mut().unwrap().select_session(Some(id));
    fixture.core.report_search(id, TOKEN, SearchReport::Started(None));
    let zoomed = fixture.send(
        Command::Surface(SurfaceCmd::Zoom),
        None,
        args(|a| a.mode = Some("show".into())),
    );
    assert!(zoomed.ok, "{zoomed:?}");
    assert_eq!(probe.actions(), ["end_search"]);
}

// MARK: - surface.cursor

fn surface_id(id: Uuid, kind: &str) -> String {
    format!("surface:{}:{kind}", wire(id))
}

#[test]
fn cursor_reads_the_addressed_or_active_surface() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    probe.set_cursor_column(Some(5));
    let left = surface_id(id, "left");
    let response = fixture.send(Command::Surface(SurfaceCmd::Cursor), Some(&left), args(|_| {}));
    assert_eq!(result(&response), json!({"id": left, "cursor": {"column": 5}}));
    let response = fixture.send(
        Command::Surface(SurfaceCmd::Cursor),
        Some(&wire(id)),
        args(|a| a.pane_id = Some(TOKEN.into())),
    );
    assert_eq!(result(&response)["id"], left);
    // the active surface is the selected session's; select the one with the terminal
    fixture.session(SessionCmd::Select, Some(&wire(id)), args(|_| {}));
    let response = fixture.send(Command::Surface(SurfaceCmd::Cursor), None, args(|_| {}));
    assert_eq!(result(&response)["id"], left);
    probe.set_cursor_column(None);
    assert_eq!(
        fixture.send(Command::Surface(SurfaceCmd::Cursor), Some(&left), args(|_| {})),
        failure("failed to read cursor position")
    );
    probe.set_realized(false);
    assert_eq!(
        fixture.send(Command::Surface(SurfaceCmd::Cursor), Some(&left), args(|_| {})),
        failure("surface not realized")
    );
}

#[test]
fn cursor_refuses_as_upstream() {
    let (mut fixture, id, _probe) = fixture_with_terminal();
    let sid = wire(id);
    let cases = [
        (UNKNOWN.to_owned(), None, "invalid surface: zzz".to_owned()),
        (sid.clone(), None, format!("invalid surface: {sid}")),
        (
            format!("surface:{sid}:bogus"),
            None,
            format!("invalid surface: surface:{sid}:bogus"),
        ),
        (
            surface_id(id, "right"),
            None,
            format!("surface not available: surface:{sid}:right"),
        ),
        (
            surface_id(id, "scratch"),
            None,
            format!("surface not available: surface:{sid}:scratch"),
        ),
        (sid.clone(), Some("nope"), "unknown pane id: nope".to_owned()),
        (
            surface_id(id, "left"),
            Some("nope"),
            "surface.cursor: --pane-id takes a session target".to_owned(),
        ),
        (
            "quick".to_owned(),
            Some("nope"),
            "surface.cursor: --pane-id takes a session target".to_owned(),
        ),
        ("quick".to_owned(), None, "surface not available: quick".to_owned()),
        (
            format!("surface:{}:left", wire(Uuid::from_u128(9))),
            None,
            format!("no such surface: surface:{}:left", wire(Uuid::from_u128(9))),
        ),
    ];
    for (target, pane_id, error) in cases {
        let response = fixture.send(
            Command::Surface(SurfaceCmd::Cursor),
            Some(&target),
            args(|a| a.pane_id = pane_id.map(str::to_owned)),
        );
        assert_eq!(response, failure(&error), "{target}");
    }
    // a session whose main pane has no terminal yet
    let other = surface_id(fixture.sessions[1], "left");
    assert_eq!(
        fixture.send(Command::Surface(SurfaceCmd::Cursor), Some(&other), args(|_| {})),
        failure(&format!("surface not available: {other}"))
    );
}

/// `--pane-id` names any pane of the session by its terminal's token, the split's and the scratch's included.
#[test]
fn cursor_by_pane_id_reaches_the_split_and_the_scratch() {
    let (mut fixture, id, _probe) = fixture_with_terminal();
    let store = fixture.store_mut().unwrap();
    store.set_split_visibility(id, true, None);
    let session = store.session_mut(id).unwrap();
    let right = FakeSurface::new("TOKEN-R");
    right.probe().set_cursor_column(Some(7));
    session.split_surface.replace(Box::new(right));
    let scratch = FakeSurface::new("TOKEN-S");
    scratch.probe().set_cursor_column(Some(3));
    session.scratch_surface.replace(Box::new(scratch));
    let sid = wire(id);
    for (token, kind, column) in [("TOKEN-R", "right", 7), ("TOKEN-S", "scratch", 3)] {
        let response = fixture.send(
            Command::Surface(SurfaceCmd::Cursor),
            Some(&sid),
            args(|a| a.pane_id = Some(token.into())),
        );
        assert_eq!(
            result(&response),
            json!({"id": surface_id(id, kind), "cursor": {"column": column}}),
            "{token}"
        );
    }
}

/// Upstream's `resolveSurfaceOwner`: a `--window` must hold the surface's session.
#[test]
fn a_surface_with_a_window_must_belong_to_it() {
    let (mut fixture, id, probe) = fixture_with_terminal();
    probe.set_cursor_column(Some(4));
    let left = surface_id(id, "left");
    let here = wire(fixture.window);
    let cursor = |fixture: &mut Fixture, window: &str| {
        fixture.send(
            Command::Surface(SurfaceCmd::Cursor),
            Some(&left),
            args(|a| a.window = Some(window.to_owned())),
        )
    };
    assert_eq!(
        result(&cursor(&mut fixture, &here)),
        json!({"id": left, "cursor": {"column": 4}})
    );
    let zoom = fixture.send(
        Command::Surface(SurfaceCmd::Zoom),
        Some(&left),
        args(|a| {
            a.window = Some(here.clone());
            a.mode = Some("show".into());
        }),
    );
    assert_eq!(result(&zoom), json!({"id": left}));
    let other = wire(fixture.core.library_mut().new_window(None));
    assert_eq!(
        cursor(&mut fixture, &other),
        failure(&format!("no such surface: {left}"))
    );
}

// MARK: - surface.zoom

fn zoomed(fixture: &mut Fixture) -> Option<String> {
    let tree = fixture.core.tree(None).unwrap();
    tree.zoomed_surface
}

#[test]
fn zoom_shows_hides_and_toggles_a_surface() {
    let (mut fixture, id, _probe) = fixture_with_terminal();
    let selected = fixture.selected().unwrap();
    let zoom = |fixture: &mut Fixture, target: Option<&str>, mode: Option<&str>| {
        fixture.send(
            Command::Surface(SurfaceCmd::Zoom),
            target,
            args(|a| a.mode = mode.map(str::to_owned)),
        )
    };
    // hiding with nothing zoomed is a bare ok
    assert_eq!(zoom(&mut fixture, None, Some("hide")), Response::success());
    let active = surface_id(selected, "left");
    assert_eq!(result(&zoom(&mut fixture, None, Some("show"))), json!({"id": active}));
    assert_eq!(zoomed(&mut fixture), Some(active.clone()));
    // an explicit id retargets; toggling it again unzooms
    let left = surface_id(id, "left");
    assert_eq!(result(&zoom(&mut fixture, Some(&left), None)), json!({"id": left}));
    assert_eq!(zoomed(&mut fixture), Some(left.clone()));
    assert!(zoom(&mut fixture, Some(&left), None).ok);
    assert_eq!(zoomed(&mut fixture), None);
    // a zoom whose session closes is dropped
    assert!(zoom(&mut fixture, Some(&left), Some("show")).ok);
    assert!(fixture.session(SessionCmd::Close, Some(&wire(id)), args(|_| {})).ok);
    let _ = fixture.core.commit();
    assert_eq!(zoomed(&mut fixture), None);
}

#[test]
fn zoom_refuses_as_upstream() {
    let (mut fixture, id, _probe) = fixture_with_terminal();
    let zoom = |fixture: &mut Fixture, target: &str, mode: &str| {
        fixture.send(
            Command::Surface(SurfaceCmd::Zoom),
            Some(target),
            args(|a| a.mode = Some(mode.into())),
        )
    };
    assert_eq!(
        zoom(&mut fixture, "active", "on"),
        failure("invalid surface zoom mode: on")
    );
    assert_eq!(
        zoom(&mut fixture, "quick", "show"),
        failure("surface not available: quick")
    );
    assert_eq!(zoom(&mut fixture, UNKNOWN, "show"), failure("invalid surface: zzz"));
    let right = surface_id(id, "right");
    assert_eq!(
        zoom(&mut fixture, &right, "show"),
        failure(&format!("surface not available: {right}"))
    );
    // hide never checks availability
    assert!(zoom(&mut fixture, &right, "hide").ok);
}

/// With a zoom up the `active` form acts on the zoomed surface, wherever the selection is.
#[test]
fn the_active_zoom_toggles_the_zoomed_surface() {
    let (mut fixture, id, _probe) = fixture_with_terminal();
    let left = surface_id(id, "left");
    let zoom = |fixture: &mut Fixture, target: Option<&str>, mode: Option<&str>| {
        fixture.send(
            Command::Surface(SurfaceCmd::Zoom),
            target,
            args(|a| a.mode = mode.map(str::to_owned)),
        )
    };
    assert!(zoom(&mut fixture, Some(&left), Some("show")).ok);
    assert_ne!(fixture.selected(), Some(id), "another session is selected");
    assert_eq!(result(&zoom(&mut fixture, None, Some("show"))), json!({"id": left}));
    assert_eq!(zoomed(&mut fixture), Some(left.clone()));
    assert_eq!(result(&zoom(&mut fixture, Some("active"), None)), json!({"id": left}));
    assert_eq!(zoomed(&mut fixture), None);
}

#[test]
fn typing_poll_reports_a_pane_token_that_disappeared() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    let probe = install(&mut fixture, id, FakeSurface::new(TOKEN).with_realized(false));
    let request = request(Command::Session(SessionCmd::Type), Some(&wire(id)), |a| {
        a.text = Some("late".to_owned());
        a.pane_id = Some(TOKEN.to_owned());
    });
    let pending = fixture.core.begin(&request);
    assert!(matches!(pending, Dispatched::Pending { .. }));
    fixture.store_mut().unwrap().session_mut(id).unwrap().surface.teardown();
    let response = answer(resume(&mut fixture.core, pending));
    assert_eq!(response, failure("unknown pane id: TOKEN-L"));
    assert!(probe.typed().is_empty());
}

#[test]
fn typing_poll_refuses_a_pane_that_became_covered_before_it_realized() {
    let mut fixture = Fixture::new(0);
    let id = fixture.sessions[0];
    let identity = fixture.store().session(id).unwrap().pane_identity;
    let token = wire(identity);
    let probe = install(&mut fixture, id, FakeSurface::new(&token).with_realized(false));
    let request = request(Command::Session(SessionCmd::Type), Some(&wire(id)), |a| {
        a.text = Some("late".to_owned());
    });
    let pending = fixture.core.begin(&request);
    assert!(matches!(pending, Dispatched::Pending { .. }));
    fixture
        .core
        .live
        .leads
        .begin(identity, crate::live::lead::LeadAttachment::new(false), true);
    probe.set_realized(true);
    let response = answer(resume(&mut fixture.core, pending));
    assert_eq!(
        response,
        failure("pane is in use on the Mac it runs on; take the lead to drive it from here")
    );
    assert!(probe.typed().is_empty());
}
