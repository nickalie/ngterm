//! `session.companion.duplicate` on the fakes: upstream's `ControlDispatcherCompanionsTests.
//! companionDuplicateRoutesTargetAndWindow`, `ControlServerCompanionsTests`' two duplicate refusals, and the copy of
//! each pane's program read through a fake process table.

use ngterm_host::ProcessStat;
use ngterm_host::fakes::{FakeProcessTable, FakeSurface};
use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Args, Response};
use serde_json::json;
use uuid::Uuid;

use super::{Fixture, UNKNOWN, answered_id, args, failure, wire};
use crate::app_actions::{ActionEffect, ActionOrigin};
use crate::hud::HudSpec;
use crate::keymap::BuiltinAction;
use crate::model::{NewSession, OverlayPane, OverlayProgram, SessionContent, StatusPane};
use crate::sidebar_actions::SidebarAction;

fn duplicate(fixture: &mut Fixture, target: Uuid, args: Args) -> Response {
    fixture.session(SessionCmd::CompanionDuplicate, Some(&wire(target)), args)
}

fn stat(pid: i32, parent: i32, pgrp: i32, session: i32) -> ProcessStat {
    ProcessStat {
        pid,
        ppid: parent,
        pgrp,
        session,
    }
}

/// Puts a realized terminal reporting foreground group `pgid` in `session`'s `pane`.
fn mount(fixture: &mut Fixture, session: Uuid, pane: StatusPane, pgid: u32) {
    let target = fixture
        .store_mut()
        .and_then(|store| store.session_mut(session))
        .unwrap();
    let surface = FakeSurface::new(Uuid::new_v4().to_string());
    surface.probe().set_realized(true);
    surface.probe().set_foreground_pid(Some(pgid));
    let _ = target.pane_slot_mut(pane).replace(Box::new(surface));
}

fn ids(fixture: &Fixture) -> Vec<Uuid> {
    fixture.order(fixture.workspace)
}

fn program(command: &str) -> OverlayProgram {
    OverlayProgram {
        command: command.to_owned(),
        ..OverlayProgram::default()
    }
}

/// A shell at its prompt in `directory`, as process 100.
fn idle_shell(directory: &str) -> FakeProcessTable {
    let table = FakeProcessTable::default();
    table.insert(stat(100, 1, 100, 100), Some(&["-sh"]));
    table.set_working_directory(100, directory);
    table
}

/// Upstream's `companionDuplicateRoutesTargetAndWindow`: target and window pick the source, `noSelect` keeps the
/// selection; a shell at its prompt copies as a plain shell where it sits.
#[test]
fn the_duplicate_takes_its_target_window_and_no_select() {
    let mut fixture = Fixture::new(1);
    let (source, other) = (fixture.sessions[0], fixture.sessions[1]);
    let directory = std::env::temp_dir().display().to_string();
    fixture
        .core
        .set_process_table(Box::new(idle_shell(&directory)), Some("sh".to_owned()));
    mount(&mut fixture, source, StatusPane::Left, 100);
    fixture.store_mut().unwrap().select_session(Some(other));

    let window = wire(fixture.window);
    let response = duplicate(
        &mut fixture,
        source,
        args(|a| {
            a.window = Some(window);
            a.no_select = Some(true);
        }),
    );
    let copy = answered_id(&response).expect("a copy");
    assert_eq!(response.result.as_ref().unwrap()["parent"], json!(wire(source)));
    assert_eq!(ids(&fixture), [source, copy, other]);
    assert_eq!(fixture.selected(), Some(other));
    let session = fixture.store().session(copy).unwrap();
    assert_eq!(session.initial_cwd(), directory);
    assert_eq!(session.launch.command, None);
    assert!(!session.split.exists);

    let selected = answered_id(&duplicate(&mut fixture, source, Args::default())).expect("a second copy");
    assert_eq!(fixture.selected(), Some(selected));
    assert_eq!(ids(&fixture), [source, copy, selected, other]);
}

/// Upstream's `testDuplicateAsCompanionRefusesAPageOrUnrealizedSourceAndCreatesNothing`.
#[test]
fn a_page_or_unrealized_source_is_refused_and_nothing_is_created() {
    let mut fixture = Fixture::new(0);
    let parent = fixture.sessions[0];
    let page = fixture
        .store_mut()
        .and_then(|store| {
            let workspace = store.workspaces().first()?.id();
            store.add_session(
                workspace,
                NewSession {
                    parent: Some(parent),
                    content: SessionContent::Page,
                    select: false,
                    ..NewSession::at_cwd("/tmp")
                },
            )
        })
        .unwrap();
    let before = ids(&fixture);
    for (id, error) in [
        (page, "a page companion has no terminal to duplicate"),
        (parent, "left pane is not realized; nothing was created"),
    ] {
        assert_eq!(duplicate(&mut fixture, id, Args::default()), failure(error));
    }
    let unknown = fixture.session(SessionCmd::CompanionDuplicate, Some(UNKNOWN), Args::default());
    assert_eq!(unknown, failure(&format!("no such session: {UNKNOWN}")));
    assert_eq!(ids(&fixture), before);
}

/// Upstream's `testDuplicateAsCompanionRefusesAPaneUnderAVisibleProgramOverlayAndCreatesNothing`, and a split's right
/// pane refused by name.
#[test]
fn a_pane_under_a_visible_program_overlay_is_refused_and_nothing_is_created() {
    let mut fixture = Fixture::new(2);
    let [covered, pane_covered, split] = [0, 1, 2].map(|at| fixture.sessions[at]);
    let store = fixture.store_mut().unwrap();
    assert!(store.open_overlay(covered, program("top")));
    store
        .open_pane_overlay(pane_covered, OverlayPane::Left, program("top"))
        .unwrap();
    store.set_split_visibility(split, true, None);
    let before = ids(&fixture);

    assert_eq!(
        duplicate(&mut fixture, covered, Args::default()),
        failure("a program overlay covers the session; nothing was created")
    );
    assert_eq!(
        duplicate(&mut fixture, pane_covered, Args::default()),
        failure("left pane is covered by a program overlay; nothing was created")
    );
    fixture
        .core
        .set_process_table(Box::new(idle_shell("/")), Some("sh".to_owned()));
    mount(&mut fixture, split, StatusPane::Left, 100);
    assert_eq!(
        duplicate(&mut fixture, split, Args::default()),
        failure("right pane is not realized; nothing was created")
    );
    assert_eq!(ids(&fixture), before);

    fixture
        .store_mut()
        .unwrap()
        .set_overlays_hidden(covered, None, true)
        .unwrap();
    assert_eq!(
        duplicate(&mut fixture, covered, Args::default()),
        failure("left pane is not realized; nothing was created"),
        "a hidden overlay does not block the copy"
    );
}

/// A refused pane names itself and the copy is not made: here the right pane's foreground cannot be read.
#[test]
fn a_pane_whose_foreground_cannot_be_read_is_refused_by_name() {
    let mut fixture = Fixture::new(0);
    let source = fixture.sessions[0];
    fixture
        .core
        .set_process_table(Box::new(idle_shell("/")), Some("sh".to_owned()));
    fixture.store_mut().unwrap().set_split_visibility(source, true, None);
    mount(&mut fixture, source, StatusPane::Left, 100);
    mount(&mut fixture, source, StatusPane::Right, 999);
    let before = ids(&fixture);
    assert_eq!(
        duplicate(&mut fixture, source, Args::default()),
        failure("right pane: the pane's foreground program cannot be read; nothing was created")
    );
    assert_eq!(ids(&fixture), before);
}

/// A split whose right pane runs a program started by a bare name: the copy keeps the axis and ratio, starts a plain
/// shell on the left and the program again on the right, by the path its own `PATH` found it at, focused left.
#[cfg(unix)]
#[test]
fn a_split_copies_its_layout_and_starts_each_panes_program_again() {
    use std::os::unix::fs::PermissionsExt as _;

    use crate::model::SplitAxis;
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let tool = bin.join("tool");
    std::fs::write(&tool, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let work = dir.path().display().to_string();
    let tool = tool.display().to_string();

    let mut fixture = Fixture::new(0);
    let source = fixture.sessions[0];
    let table = idle_shell(&work);
    table.insert(stat(200, 1, 200, 200), Some(&["-sh"]));
    table.insert(stat(201, 200, 201, 200), Some(&["tool", "two words"]));
    table.set_working_directory(201, &work);
    table.set_executable_path(201, &tool);
    table.set_search_path(201, &format!("relative:{}", bin.display()));
    fixture.core.set_process_table(Box::new(table), Some("sh".to_owned()));
    let store = fixture.store_mut().unwrap();
    store.set_split_visibility(source, true, Some(SplitAxis::TopBottom));
    store.apply_split_ratio(0.3, source);
    mount(&mut fixture, source, StatusPane::Left, 100);
    mount(&mut fixture, source, StatusPane::Right, 201);

    let copy = answered_id(&duplicate(&mut fixture, source, Args::default())).expect("a copy");
    let session = fixture.store().session(copy).unwrap();
    assert_eq!(session.parent_id(), Some(source));
    assert_eq!(session.initial_cwd(), work);
    assert_eq!(session.launch.command, None);
    assert_eq!(session.split.axis, SplitAxis::TopBottom);
    assert_eq!(session.split.ratio, Some(0.3));
    assert_eq!(session.initial_split_cwd.as_deref(), Some(work.as_str()));
    assert_eq!(
        session.split_launch.command.as_deref(),
        Some(format!("'{tool}' 'two words'").as_str())
    );
    assert!(!session.split_launch.wait);
    assert!(!session.focuses_split());
}

/// A program whose image cannot be told from its starting path is refused rather than copied.
#[test]
fn a_program_not_found_where_it_was_started_is_refused() {
    let mut fixture = Fixture::new(0);
    let source = fixture.sessions[0];
    let table = FakeProcessTable::default();
    table.insert(stat(201, 1, 201, 1), Some(&["no-such-tool-anywhere"]));
    table.set_working_directory(201, &std::env::temp_dir().display().to_string());
    table.set_executable_path(201, "/no/such/image");
    table.set_search_path(201, "/no/such/dir");
    fixture.core.set_process_table(Box::new(table), Some("sh".to_owned()));
    mount(&mut fixture, source, StatusPane::Left, 201);
    assert_eq!(
        duplicate(&mut fixture, source, Args::default()),
        failure("left pane: the path the foreground program was started from cannot be found; nothing was created")
    );
}

/// Upstream's `testARefusedDuplicateShowsTheReasonInATimedHudOverTheSession`, through the keymap action.
#[test]
fn a_refused_duplicate_shows_the_reason_in_a_timed_hud_over_the_session() {
    let mut fixture = Fixture::new(0);
    let bodies = tempfile::tempdir().unwrap();
    // the panel is a HUD, which needs the painter
    fixture.core.set_hud_body_dir(bodies.path().to_path_buf());
    let source = fixture.sessions[0];
    let before = ids(&fixture);
    let origin = ActionOrigin {
        window: fixture.window,
        quick: false,
    };
    assert_eq!(
        fixture.core.perform_builtin(origin, BuiltinAction::DuplicateCompanion),
        ActionEffect::Done
    );
    let hud = fixture
        .store()
        .session(source)
        .and_then(|session| session.overlay.effective_hud_spec());
    let expected = HudSpec {
        hide_after: Some(5.0),
        ..HudSpec::new("Duplicate as Companion: left pane is not realized; nothing was created")
    };
    assert_eq!(hud, Some(expected));
    assert_eq!(ids(&fixture), before);
}

/// The row menu's Duplicate as Companion selects the copy and hands its terminal the keyboard.
#[test]
fn the_row_menus_duplicate_selects_the_copy() {
    let mut fixture = Fixture::new(1);
    let (source, other) = (fixture.sessions[0], fixture.sessions[1]);
    fixture
        .core
        .set_process_table(Box::new(idle_shell("/")), Some("sh".to_owned()));
    mount(&mut fixture, source, StatusPane::Left, 100);
    fixture.store_mut().unwrap().select_session(Some(other));
    let window = fixture.window;
    assert!(
        fixture
            .core
            .sidebar_action(window, SidebarAction::DuplicateAsCompanion(source))
    );
    let copy = fixture.selected().unwrap();
    assert!(fixture.core.pane_focus_requested(copy));
    assert_eq!(ids(&fixture), [source, copy, other]);
    assert_eq!(fixture.store().session(copy).unwrap().parent_id(), Some(source));
}

/// Runs new panes in Live sessions whose daemons start `login_shell`.
#[cfg(unix)]
fn install_live(fixture: &mut Fixture, resources: &std::path::Path, login_shell: &str) {
    use std::sync::Arc;

    use ngterm_host::SessionHost;
    use ngterm_host::fakes::FakeSessionHost;
    use ngterm_proto::ControlLiveEndpoint;

    use crate::LiveInstall;
    use crate::live::support::Inputs;
    use crate::persist::RestoreMode;
    use crate::restore::mode::RestoreLaunchDecision;
    fixture
        .core
        .set_restore_launch(RestoreLaunchDecision::new(RestoreMode::Live, None));
    fixture.core.install_live_sessions(LiveInstall {
        host: Arc::new(FakeSessionHost::new()) as Arc<dyn SessionHost>,
        endpoint: ControlLiveEndpoint {
            executable: "/opt/ngterm/bin/ngterm-session".to_owned(),
            socket_directory: "/tmp/ngterm-live-test".to_owned(),
        },
        wrap: Inputs {
            executable: crate::live::test_executable(resources, "sh"),
            login_shell: Some(login_shell.to_owned()),
            resources_dir: Some(resources.to_path_buf()),
            state_dir: resources.join("state"),
            ..Inputs::default()
        },
        state_dir: resources.to_path_buf(),
        outdated_before: None,
    });
}

/// Under Live too the copy stores upstream's POSIX line with its argv, whatever the login shell (fish evaluates the
/// POSIX line alike, PowerShell gets the call operator at launch, `crate::restore::spawn_plan`); a login shell that
/// cannot carry the argv refuses the copy.
#[cfg(unix)]
#[test]
fn a_live_copy_stores_the_posix_line_and_its_argv() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let tool = dir.path().join("tool");
    std::fs::write(&tool, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let work = dir.path().display().to_string();
    let tool = tool.display().to_string();
    let copy_under = |login_shell: &str| {
        let mut fixture = Fixture::new(0);
        let source = fixture.sessions[0];
        install_live(&mut fixture, dir.path(), login_shell);
        let table = FakeProcessTable::default();
        table.insert(stat(201, 1, 201, 1), Some(&[tool.as_str(), "it's"]));
        table.set_working_directory(201, &work);
        table.set_executable_path(201, &tool);
        fixture.core.set_process_table(Box::new(table), Some("sh".to_owned()));
        mount(&mut fixture, source, StatusPane::Left, 201);
        let response = duplicate(&mut fixture, source, Args::default());
        let launch = answered_id(&response).map(|copy| fixture.store().session(copy).unwrap().launch.clone());
        (response, launch)
    };
    let posix = format!("'{tool}' 'it'\\''s'");
    for login_shell in ["/usr/bin/pwsh", "/usr/bin/fish", "/bin/zsh"] {
        let (response, launch) = copy_under(login_shell);
        let launch = launch.unwrap_or_else(|| panic!("{login_shell}: {response:?}"));
        assert_eq!(launch.command.as_deref(), Some(posix.as_str()), "{login_shell}");
        assert_eq!(
            launch.argv,
            Some(vec![tool.clone(), "it's".to_owned()]),
            "{login_shell}"
        );
    }
    let (response, _) = copy_under("/usr/bin/nu");
    assert_eq!(
        response,
        failure("left pane: the foreground program's arguments cannot be replayed; nothing was created")
    );
}

/// The stored line is the ordinary shell's (`/bin/sh -c`'s, `cmd.exe`'s), and the Live login shell new panes run in
/// must carry the argv too: a Windows program path with a `%` has no `cmd.exe` line, under Live as outside it.
#[test]
fn the_creation_line_needs_both_the_ordinary_shell_and_the_live_one() {
    use crate::commands::companion_duplicate::creation_line;
    use crate::shell_dialect::ShellDialect;

    let words = |argv: &[&str]| argv.iter().map(|word| (*word).to_owned()).collect::<Vec<_>>();
    let node = words(&[r"C:\Program Files\nodejs\node.exe", "app.js"]);
    let cmd = Some(r#""C:\Program Files\nodejs\node.exe" app.js"#.to_owned());
    assert_eq!(creation_line(ShellDialect::Cmd, None, &node), cmd);
    assert_eq!(creation_line(ShellDialect::Cmd, Some("pwsh.exe"), &node), cmd);
    let percent = words(&[r"C:\100%\tool.exe", "x"]);
    assert_eq!(creation_line(ShellDialect::Cmd, Some("pwsh.exe"), &percent), None);
    assert_eq!(creation_line(ShellDialect::Cmd, None, &percent), None);
    let spaced = words(&["/usr/bin/tool", "a b"]);
    assert_eq!(
        creation_line(ShellDialect::Posix, Some("/usr/bin/fish"), &spaced).as_deref(),
        Some("'/usr/bin/tool' 'a b'")
    );
    assert_eq!(creation_line(ShellDialect::Posix, Some("/usr/bin/nu"), &spaced), None);
}
