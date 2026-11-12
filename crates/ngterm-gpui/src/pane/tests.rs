//! What a pane runs (ported from the GTK build's `deck.rs` spec tests) and what its terminal's events report.

use std::path::PathBuf;

use ngterm_core::model::{NewSession, WindowLibrary};
use ngterm_term::{ExitStatus, PaneEvent};

use super::*;

/// A pane program that prints nothing of its own and runs until its input ends, the terminal echoing what is typed:
/// `cat` (`/bin/sh -c 'exec cat'`), and on Windows `findstr`, which reads its input line by line in the console's
/// line mode and repeats every line.
pub(crate) fn idle_program() -> ngterm_term::PaneCommand {
    if cfg!(windows) {
        ngterm_term::PaneCommand::new("findstr.exe").args(["/R", "^"])
    } else {
        ngterm_term::PaneCommand::new("/bin/sh").arg("-c").arg("exec cat")
    }
}

/// What ends [`idle_program`]'s input when typed: Ctrl+D, and on Windows Ctrl+Z and Enter.
pub(crate) const IDLE_EOF: &[u8] = if cfg!(windows) { b"\x1a\r" } else { b"\x04" };

/// [`idle_program`] with the inherited `NGTERM_*`/`AGTERM_*` variables removed.
pub(crate) fn idle_command() -> ngterm_term::PaneCommand {
    let mut command = idle_program();
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("NGTERM_") || name.starts_with("AGTERM_") {
            command = command.env_remove(key);
        }
    }
    command
}

fn context() -> PaneContext {
    PaneContext {
        socket: "/tmp/a.sock".to_owned(),
        program_version: "1.2.3".to_owned(),
    }
}

#[test]
fn remote_clipboard_destination_follows_both_session_panes() {
    let mut session = Session::new(ngterm_core::model::SessionSeed {
        initial_cwd: "/srv".to_owned(),
        remote_host: Some("user@remote".to_owned()),
        split_pane_identity: Some(Uuid::new_v4()),
        ..ngterm_core::model::SessionSeed::default()
    });
    let (window, workspace) = (Uuid::new_v4(), Uuid::new_v4());
    for spec in [
        PaneSpec::left(window, workspace, &session),
        PaneSpec::split(window, workspace, &session).unwrap(),
    ] {
        assert_eq!(spec.terminal_spec().remote_host.as_deref(), Some("user@remote"));
    }
    // Scratch panes run a local shell even when the session was attached from another machine.
    assert_eq!(
        PaneSpec::scratch(window, workspace, &mut session)
            .terminal_spec()
            .remote_host,
        None
    );
}

/// A window of one workspace holding three sessions, the first selected.
pub(crate) fn three_sessions() -> (WindowLibrary, Uuid, Vec<Uuid>) {
    let mut library = WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let store = library.store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let first = store.sessions().next().unwrap().id();
    let second = store.add_session(workspace, NewSession::at_cwd("/srv")).unwrap();
    let third = store
        .add_session(
            workspace,
            NewSession {
                command: Some("htop".to_owned()),
                wait: true,
                select: false,
                ..NewSession::at_cwd("/var")
            },
        )
        .unwrap();
    store.select_session(Some(first));
    (library, window, vec![first, second, third])
}

#[test]
fn the_page_env_names_the_session_its_window_workspace_and_pane() {
    let (library, window, ids) = three_sessions();
    let store = library.store(window).unwrap();
    let workspace = store.workspaces()[0].id();
    for id in ids {
        let spec = PaneSpec::primary(window, store, id).unwrap();
        let session = store.session(id).unwrap();
        let env = spec.env(&context());
        assert_eq!(env["NGTERM_SESSION_ID"], pane_env::uuid_string(id));
        assert_eq!(env["NGTERM_WINDOW_ID"], pane_env::uuid_string(window));
        assert_eq!(env["NGTERM_WORKSPACE_ID"], pane_env::uuid_string(workspace));
        assert_eq!(env["NGTERM_PANE"], "left");
        assert_eq!(env["NGTERM_PANE_ID"], pane_env::uuid_string(session.pane_identity));
        assert_eq!(spec.token, env["NGTERM_PANE_ID"]);
        assert_eq!(env["NGTERM_SOCKET"], "/tmp/a.sock");
        assert_eq!(env["TERM_PROGRAM"], "ngterm");
        assert_eq!(env["TERM_PROGRAM_VERSION"], "1.2.3");
    }
}

#[test]
fn a_session_pane_launches_its_own_command_in_its_cwd_once_seeded() {
    let (library, window, ids) = three_sessions();
    let store = library.store(window).unwrap();
    for id in ids {
        let spec = PaneSpec::primary(window, store, id).unwrap();
        let session = store.session(id).unwrap();
        let launch = spec.launch(&context());
        assert_eq!(launch.working_directory, Some(PathBuf::from(session.effective_cwd())));
        assert_eq!(launch.command, session.launch.command);
        assert_eq!(launch.wait_after_command, session.launch.wait);
        assert!(
            launch
                .env
                .iter()
                .any(|(key, value)| key == "NGTERM_PANE_ID" && *value == spec.token)
        );
        assert!(spec.gated(), "a session pane waits for its seed");
    }
}

#[test]
fn a_session_outside_the_tree_has_no_page_spec() {
    let (library, window, _) = three_sessions();
    assert_eq!(
        PaneSpec::primary(window, library.store(window).unwrap(), Uuid::new_v4()),
        None
    );
}

#[test]
fn the_split_pane_runs_in_its_own_cwd_with_its_identity_and_command() {
    let (mut library, window, ids) = three_sessions();
    let store = library.store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    assert_eq!(
        PaneSpec::split(window, workspace, store.session(ids[0]).unwrap()),
        None,
        "no split yet"
    );
    store.set_split_visibility(ids[0], true, None);
    let session = store.session_mut(ids[0]).unwrap();
    session.split_cwd = Some("/srv/right".to_owned());
    session.split_launch.command = Some("htop".to_owned());
    session.split_launch.wait = true;
    let session = store.session(ids[0]).unwrap();
    let spec = PaneSpec::split(window, workspace, session).unwrap();
    let env = spec.env(&context());
    assert_eq!(env["NGTERM_PANE"], "right");
    let identity = pane_env::uuid_string(session.split_pane_identity.unwrap());
    assert_eq!(env["NGTERM_PANE_ID"], identity);
    assert_eq!(spec.token, identity);
    let launch = spec.launch(&context());
    assert_eq!(launch.working_directory, Some(PathBuf::from("/srv/right")));
    assert_eq!(launch.command.as_deref(), Some("htop"));
    assert!(launch.wait_after_command);
    assert_eq!(
        PaneSpec::of_token(window, store, ids[0], &identity),
        None,
        "no terminal carries the token yet"
    );
    store
        .session_mut(ids[0])
        .unwrap()
        .split_surface
        .replace(Box::new(ngterm_host::fakes::FakeSurface::new(identity.clone())));
    assert_eq!(
        PaneSpec::of_token(window, store, ids[0], &identity),
        Some(spec),
        "a replacement inherits the pane's own spec"
    );
}

#[test]
fn remote_pane_launches_use_a_local_directory_after_remote_pwd_reports() {
    let mut session = Session::new(ngterm_core::model::SessionSeed {
        initial_cwd: "/local/home".to_owned(),
        remote_host: Some("buildbox".to_owned()),
        split_pane_identity: Some(Uuid::new_v4()),
        ..ngterm_core::model::SessionSeed::default()
    });
    session.report_pwd(false, "/only/on/remote/left");
    session.report_pwd(true, "/only/on/remote/right");
    let (window, workspace) = (Uuid::new_v4(), Uuid::new_v4());
    for spec in [
        PaneSpec::left(window, workspace, &session),
        PaneSpec::split(window, workspace, &session).unwrap(),
    ] {
        assert_eq!(
            spec.launch(&context()).working_directory,
            Some(PathBuf::from("/local/home"))
        );
    }
}

#[test]
fn each_scratch_terminal_gets_a_fresh_token_and_runs_its_command_once() {
    let (mut library, window, ids) = three_sessions();
    let store = library.store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let session = store.session_mut(ids[1]).unwrap();
    session.scratch_command = Some("lazygit".to_owned());
    let first = PaneSpec::scratch(window, workspace, session);
    assert_eq!(first.command.as_deref(), Some("lazygit"));
    assert_eq!(session.scratch_command, None, "taken: it runs once");
    let env = first.env(&context());
    assert_eq!(env["NGTERM_PANE"], "scratch");
    assert_eq!(env["NGTERM_PANE_ID"], first.token);
    assert_eq!(first.cwd, "/srv");
    assert!(!first.wait && !first.gated(), "the scratch starts from its spec");
    let second = PaneSpec::scratch(window, workspace, session);
    assert_eq!(second.command, None, "a respawn is a login shell");
    assert_ne!(second.token, first.token);
    assert_eq!(
        PaneSpec::of_token(window, store, ids[1], &first.token),
        None,
        "no replacement spec for a scratch"
    );
}

#[test]
fn a_seed_runs_over_the_panes_own_environment() {
    let (library, window, ids) = three_sessions();
    let spec = PaneSpec::primary(window, library.store(window).unwrap(), ids[0]).unwrap();
    let seed = LaunchSeed {
        shell_profile: None,
        command: Some("ngterm-session attach x".to_owned()),
        initial_input: Some(ngterm_core::command_restore::PaneInput::Line("ls".to_owned())),
        wait_after_command: false,
        env: vec![
            ("NGTERM_LIVE_DIR".to_owned(), "/tmp/z".to_owned()),
            ("NGTERM_PANE".to_owned(), "over".to_owned()),
        ],
        live: true,
        attach: None,
    };
    let launch = spec.seeded(&seed, &context());
    assert_eq!(launch.command.as_deref(), Some("ngterm-session attach x"));
    assert_eq!(
        launch.initial_input,
        Some(ngterm_core::command_restore::PaneInput::Line("ls".to_owned()))
    );
    let value = |key: &str| {
        launch
            .env
            .iter()
            .find(|(known, _)| known == key)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(value("NGTERM_LIVE_DIR"), Some("/tmp/z"));
    assert_eq!(value("NGTERM_PANE"), Some("over"), "the seed's variables win");
    assert_eq!(value("NGTERM_SESSION_ID").map(str::len), Some(36));
}

fn exited(held: bool) -> TerminalEvent {
    TerminalEvent::Pane(PaneEvent::Exited(ExitStatus {
        code: 0,
        signal: None,
        held,
    }))
}

#[test]
fn a_pane_events_report_what_the_gtk_build_routed() {
    let shell = Ran::default();
    let command = Ran { command: true };
    let title = TerminalEvent::Pane(PaneEvent::Title("vim".to_owned()));
    assert_eq!(
        report(&title, shell, "box", 12.0, false),
        [Report::Pane(PaneReport::Title("vim".to_owned()))]
    );
    let pwd = TerminalEvent::Pane(PaneEvent::Pwd("file://box/srv/a%20b".to_owned()));
    assert_eq!(
        report(&pwd, shell, "box", 12.0, false),
        [Report::Pane(PaneReport::Pwd("/srv/a b".to_owned()))]
    );
    let foreign = TerminalEvent::Pane(PaneEvent::Pwd("file://elsewhere/srv".to_owned()));
    assert!(
        report(&foreign, shell, "box", 12.0, false).is_empty(),
        "a foreign host is ignored"
    );
    assert_eq!(report(&exited(false), shell, "box", 12.0, false), [Report::Exited]);
    assert!(
        report(&exited(true), shell, "box", 12.0, false).is_empty(),
        "a held login shell waits for a key"
    );
    assert_eq!(
        report(&exited(true), command, "box", 12.0, false),
        [Report::Held],
        "a held command is reported held, whether its launch or the config's wait-after-command held it"
    );
    assert_eq!(report(&exited(false), command, "box", 12.0, false), [Report::Exited]);
    assert_eq!(
        report(&TerminalEvent::FontSize, shell, "box", 13.5, false),
        [Report::FontSize(13.5)]
    );
    assert_eq!(
        report(&TerminalEvent::StartSearch(String::new()), shell, "box", 12.0, false),
        [Report::Search(SearchReport::Started(None))]
    );
    assert_eq!(
        report(&TerminalEvent::StartSearch("x".to_owned()), shell, "box", 12.0, false),
        [Report::Search(SearchReport::Started(Some("x".to_owned())))]
    );
    assert_eq!(
        report(
            &TerminalEvent::Pane(PaneEvent::SearchTotal(Some(3))),
            shell,
            "box",
            12.0,
            false
        ),
        [Report::Search(SearchReport::Total(Some(3)))]
    );
    assert_eq!(
        report(
            &TerminalEvent::OpenUrl {
                url: "https://x.dev".to_owned(),
                intent: ngterm_core::link_policy::ClickIntent::Browser,
                kind: LinkKind::Osc8
            },
            shell,
            "box",
            12.0,
            false
        ),
        [Report::OpenUrl {
            url: "https://x.dev".to_owned(),
            intent: ngterm_core::link_policy::ClickIntent::Browser,
            kind: LinkKind::Osc8
        }]
    );
    assert!(report(&TerminalEvent::Pane(PaneEvent::Bell), shell, "box", 12.0, false).is_empty());
}

#[test]
fn a_key_on_a_lead_covered_pane_is_not_a_keystroke() {
    let shell = Ran::default();
    let key = TerminalEvent::Keystroke(StatusKeystroke::Interrupt);
    assert_eq!(
        report(&key, shell, "box", 12.0, false),
        [Report::Keystroke(StatusKeystroke::Interrupt)]
    );
    assert!(
        report(&key, shell, "box", 12.0, true).is_empty(),
        "the takeover key is neither user activity nor a status clear"
    );
    // a cover hides only the takeover key: the pane's other events still report
    assert_eq!(report(&exited(false), shell, "box", 12.0, true), [Report::Exited]);
}

use ngterm_core::{Core, CoreConfig, CoreState};
use ngterm_host::{MainLoop, MainLoopClosed, MainTask};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Default)]
struct Posted(Mutex<Vec<MainTask<AppState>>>);

impl MainLoop<AppState> for Posted {
    fn post(&self, task: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        self.0.lock().unwrap().push(task);
        Ok(())
    }
    fn post_after(&self, _: Duration, task: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        self.post(task)
    }
}

#[test]
fn search_selection_and_notifications_keep_their_report_payloads() {
    let event = TerminalEvent::Pane(PaneEvent::SearchSelected(Some(2)));
    assert_eq!(
        report(&event, Ran::default(), "host", 12., false),
        [Report::Search(SearchReport::Selected(Some(2)))]
    );
    let event = TerminalEvent::Pane(PaneEvent::Notification {
        title: "Title".into(),
        body: "Body".into(),
    });
    assert_eq!(
        report(&event, Ran::default(), "host", 12., false),
        [Report::Notification {
            title: "Title".into(),
            body: "Body".into()
        }]
    );
}

#[test]
fn queued_font_and_search_reports_update_the_addressed_pane() {
    let (mut library, window, ids) = three_sessions();
    let id = ids[0];
    let token = "font-report-pane";
    library
        .store_mut(window)
        .unwrap()
        .session_mut(id)
        .unwrap()
        .surface
        .replace(Box::new(ngterm_host::fakes::FakeSurface::new(token)));
    let mut state = AppState::new(Core::new(
        CoreConfig {
            app: ngterm_proto::AppIdentity::from_recorded("1.2.3", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            ..CoreState::default()
        },
    ));
    let queue = Arc::new(Posted::default());
    let main_loop: Arc<dyn MainLoop<AppState>> = queue.clone();
    let poster = Arc::downgrade(&main_loop);
    for report in [
        Report::FontSize(17.5),
        Report::Search(SearchReport::Started(Some("needle".into()))),
        Report::Search(SearchReport::Total(Some(4))),
        Report::Search(SearchReport::Selected(Some(2))),
    ] {
        post_report(
            &poster,
            id,
            token,
            report,
            crate::state::NotifyFocus::default(),
            &ExitClaim::default(),
        );
    }
    for task in std::mem::take(&mut *queue.0.lock().unwrap()) {
        task(&mut state);
    }
    let session = state.core.library().store(window).unwrap().session(id).unwrap();
    assert_eq!(session.font_size, Some(17.5));
    assert!(session.search.active);
    assert_eq!(session.search.total, Some(4));
    assert_eq!(session.search.selected, Some(2));
}
