//! File ▸ Attach Remote: `remotes.conf` as the core follows it, its problems banner, and the picker and HUD flow
//! (upstream's `RemotesWatcherTests`, `RemoteAttachTests` and `notifyRemotesDiagnostics`), over a fake ssh runner that
//! answers only when the test says so.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Command, ControlPickOutcome, ControlPickResult, Request};
use serde_json::json;
use uuid::Uuid;

use super::{Fixture, args, wire};
use crate::Core;
use crate::commands::notify::NotificationEffect;
use crate::hud::{HudSpec, HudSpinner};
use crate::modal::PendingPick;
use crate::palette_catalog::PaletteFacts;
use crate::persist::AppSettings;
use crate::remote::tree::{RemoteCommandResult, RemoteCommandRunner};
use crate::settings_model::SettingsChange;
use crate::sidebar_actions::SidebarAction;

const REMOTE_SESSION: &str = "0A0B0C0D-0000-0000-0000-0000000000B1";

// MARK: - fakes

/// Holds every remote command until the test answers it, in call order (upstream's `FakeAttacher.answerTree`).
#[derive(Default)]
struct GatedRunner {
    state: Mutex<Gate>,
    answered: Condvar,
}

#[derive(Default)]
struct Gate {
    /// The host of every command run, in order.
    hosts: Vec<String>,
    /// The remote line of every command run, in order.
    lines: Vec<String>,
    /// The answers given so far, by call.
    answers: Vec<Option<RemoteCommandResult>>,
}

impl RemoteCommandRunner for GatedRunner {
    fn run(&self, argv: &[String], _deadline: Duration) -> RemoteCommandResult {
        let mut gate = self.state.lock().unwrap();
        // the destination is the argument before the remote command line
        let host = argv.get(argv.len().saturating_sub(2)).cloned().unwrap_or_default();
        let call = gate.hosts.len();
        gate.hosts.push(host);
        gate.lines.push(argv.last().cloned().unwrap_or_default());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(answer) = gate.answers.get_mut(call).and_then(Option::take) {
                return answer;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return RemoteCommandResult {
                    status: -1,
                    stdout: String::new(),
                    stderr: "unanswered".to_owned(),
                };
            }
            gate = self.answered.wait_timeout(gate, left).unwrap().0;
        }
    }
}

impl GatedRunner {
    fn hosts(&self) -> Vec<String> {
        self.state.lock().unwrap().hosts.clone()
    }

    fn lines(&self) -> Vec<String> {
        self.state.lock().unwrap().lines.clone()
    }

    fn answer(&self, call: usize, result: RemoteCommandResult) {
        let mut gate = self.state.lock().unwrap();
        if gate.answers.len() <= call {
            gate.answers.resize(call + 1, None);
        }
        gate.answers[call] = Some(result);
        self.answered.notify_all();
    }
}

fn listing(sessions: &[serde_json::Value]) -> RemoteCommandResult {
    RemoteCommandResult {
        status: 0,
        stdout: json!({"ok": true, "result": {"remote": {
            "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
            "sessions": sessions,
        }}})
        .to_string(),
        stderr: String::new(),
    }
}

/// Upstream's `build`: one pane running vim, with a context; its daemon is one ngterm can address.
fn build_session() -> serde_json::Value {
    let daemon = crate::live::daemon_name(Uuid::from_u128(0xd1));
    json!({
        "id": REMOTE_SESSION, "name": "build", "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
        "workspaceName": "work", "context": "release prep", "cwd": "/repo",
        "panes": [{"pane": "left", "daemon": daemon, "foreground": ["/usr/bin/vim", "a.txt"]}],
        "status": "blocked", "statusColor": "#ff8800", "statusShape": "star"
    })
}

fn refused(stderr: &str) -> RemoteCommandResult {
    RemoteCommandResult {
        status: 255,
        stdout: String::new(),
        stderr: stderr.to_owned(),
    }
}

/// A window whose config directory is `config`, over a gated runner.
struct Attach {
    fixture: Fixture,
    runner: Arc<GatedRunner>,
    config: tempfile::TempDir,
    /// The window's selected session, which the attach's panels go over.
    session: Uuid,
}

impl Attach {
    fn new(remotes: Option<&str>) -> Self {
        Self::with(remotes, AppSettings::default())
    }

    fn with(remotes: Option<&str>, settings: AppSettings) -> Self {
        let config = tempfile::tempdir().unwrap();
        if let Some(text) = remotes {
            std::fs::write(config.path().join("remotes.conf"), text).unwrap();
        }
        let settings = AppSettings {
            config_directory: Some(config.path().to_string_lossy().into_owned()),
            ..settings
        };
        let mut fixture = Fixture::with_settings(0, settings);
        // the attach's panels are HUDs, which need the painter
        fixture.core.set_hud_body_dir(config.path().join("bodies"));
        let runner = Arc::new(GatedRunner::default());
        fixture.core.set_remote_runner(runner.clone());
        let session = fixture.store().selected_session_id().unwrap();
        Self {
            fixture,
            runner,
            config,
            session,
        }
    }

    fn core(&mut self) -> &mut Core {
        &mut self.fixture.core
    }

    fn pending(&self) -> Option<PendingPick> {
        self.fixture
            .core
            .modal_slot(self.fixture.window)
            .and_then(|slot| slot.pending().cloned())
    }

    /// The panel over the starting session, while one shows.
    fn hud(&self) -> Option<HudSpec> {
        self.fixture
            .store()
            .session(self.session)
            .and_then(|session| session.overlay.effective_hud_spec())
    }

    /// Ticks the flows until `done` holds.
    fn drive(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        for _ in 0..400 {
            self.fixture.core.remote_tick(Instant::now());
            if done(self) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("{what}: hud {:?}", self.hud());
    }

    /// Ticks long enough for an answered command to land, for checks that nothing happens.
    fn idle(&mut self) {
        for _ in 0..20 {
            self.fixture.core.remote_tick(Instant::now());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_for_runs(&mut self, count: usize) {
        let runner = Arc::clone(&self.runner);
        self.drive("the remote command starts", |_| runner.hosts().len() == count);
    }

    fn resolve(&mut self, pick: &PendingPick, outcome: ControlPickResult) {
        let window = self.fixture.window;
        self.fixture.core.resolve_pick(window, &pick.id, outcome);
    }

    fn cancel(&mut self, pick: &PendingPick) {
        self.resolve(pick, ControlPickResult::outcome(ControlPickOutcome::Cancelled));
    }

    fn banners(&mut self) -> Vec<String> {
        self.fixture
            .core
            .take_notifications()
            .into_iter()
            .filter_map(|effect| match effect {
                NotificationEffect::App { identity, title, body } => Some(format!("{identity}|{title}|{body}")),
                _ => None,
            })
            .collect()
    }

    fn remotes_file(&self) -> PathBuf {
        self.config.path().join("remotes.conf")
    }

    fn destinations(&self) -> Vec<String> {
        self.fixture
            .core
            .configured_remotes()
            .iter()
            .map(|entry| entry.destination.clone())
            .collect()
    }
}

fn progress(text: &str) -> HudSpec {
    HudSpec {
        spinner: Some(HudSpinner::Bar),
        hide_after: Some(30.0),
        ..HudSpec::new(format!("Attach Remote: {text}"))
    }
}

fn failure(text: &str) -> HudSpec {
    HudSpec {
        hide_after: Some(5.0),
        ..HudSpec::new(format!("Attach Remote: {text}"))
    }
}

fn replace(path: &Path, text: &str) {
    let staged = path.with_extension("tmp");
    std::fs::write(&staged, text).unwrap();
    std::fs::rename(&staged, path).unwrap();
}

// MARK: - remotes.conf and its banner

#[test]
fn the_launch_reports_a_file_already_broken_once() {
    let mut attach = Attach::new(Some("studio\n-bad\n"));
    assert_eq!(
        attach.destinations(),
        ["studio"],
        "the core reads the file it was built over"
    );
    assert!(attach.banners().is_empty(), "construction reports nothing");
    attach.core().load_remotes_at_launch();
    assert_eq!(
        attach.banners(),
        ["remotes-diagnostics|Remotes|1 issue in remotes.conf"]
    );
    attach.core().refresh_remotes(true);
    assert!(
        attach.banners().is_empty(),
        "a file that stays broken is not reported again"
    );
}

#[test]
fn a_clean_file_at_launch_posts_nothing_and_an_edit_that_breaks_it_does() {
    let mut attach = Attach::new(Some("studio\n"));
    attach.core().load_remotes_at_launch();
    assert!(attach.banners().is_empty());
    replace(&attach.remotes_file(), "-bad\n-worse\n");
    attach.core().refresh_remotes(true);
    assert_eq!(
        attach.banners(),
        ["remotes-diagnostics|Remotes|2 issues in remotes.conf"]
    );
    assert!(attach.destinations().is_empty());
}

#[test]
fn the_banner_waits_for_the_banners_setting() {
    let settings = AppSettings {
        notifications_enabled: Some(false),
        ..AppSettings::default()
    };
    let mut attach = Attach::with(Some("-bad\n"), settings);
    attach.core().load_remotes_at_launch();
    assert!(attach.banners().is_empty());
}

#[test]
fn the_menu_row_follows_the_file_once_it_is_read_again() {
    let mut attach = Attach::new(None);
    let window = attach.fixture.window;
    let has_remotes = |core: &Core| core.palette_context(window).has(PaletteFacts::HAS_REMOTES);
    assert!(!has_remotes(&attach.fixture.core), "no file lists nothing");
    std::fs::write(attach.remotes_file(), "studio\n").unwrap();
    attach.core().commit();
    assert!(
        !has_remotes(&attach.fixture.core),
        "a settle alone does not read the disk"
    );
    attach.core().refresh_remotes(true);
    assert!(has_remotes(&attach.fixture.core));
    std::fs::remove_file(attach.remotes_file()).unwrap();
    attach.core().refresh_remotes(true);
    assert!(!has_remotes(&attach.fixture.core), "a deleted file");
    assert!(attach.banners().is_empty(), "a missing file is no problem");
}

#[test]
fn a_config_directory_switch_reads_the_new_file_and_reports_nothing() {
    let mut attach = Attach::new(Some("studio\n"));
    let remotes_banners = |attach: &mut Attach| -> Vec<String> {
        attach
            .banners()
            .into_iter()
            .filter(|banner| banner.starts_with("remotes-diagnostics"))
            .collect()
    };
    assert!(remotes_banners(&mut attach).is_empty(), "a clean file at the start");
    // from a clean file to a broken one: the edge a refresh reports, which a switch must not
    let broken = attach.config.path().join("broken");
    std::fs::create_dir(&broken).unwrap();
    std::fs::write(broken.join("remotes.conf"), "mini\n-bad\n").unwrap();
    assert!(attach.core().change_settings(SettingsChange::ConfigDirectory(Some(
        broken.to_string_lossy().into_owned()
    ))));
    assert_eq!(attach.destinations(), ["mini"], "the broken file switched to");
    assert!(
        remotes_banners(&mut attach).is_empty(),
        "a switch from a clean file to a broken one reports nothing"
    );
    attach.core().refresh_remotes(true);
    assert!(
        remotes_banners(&mut attach).is_empty(),
        "the switch counted the episode: a later read of the same file reports nothing"
    );

    let later = attach.config.path().join("later/nested");
    assert!(attach.core().change_settings(SettingsChange::ConfigDirectory(Some(
        later.to_string_lossy().into_owned()
    ))));
    assert!(attach.destinations().is_empty());
    assert_eq!(attach.fixture.core.watched_remotes_path(), later.join("remotes.conf"));
    std::fs::create_dir_all(&later).unwrap();
    std::fs::write(later.join("remotes.conf"), "mini\n-bad\n").unwrap();
    attach.core().refresh_remotes(true);
    assert_eq!(
        attach.destinations(),
        ["mini"],
        "a file in a directory that did not exist"
    );
    attach.banners();

    replace(&attach.remotes_file(), "other\n-bad\n");
    let back = attach.config.path().to_string_lossy().into_owned();
    assert!(
        attach
            .core()
            .change_settings(SettingsChange::ConfigDirectory(Some(back)))
    );
    assert_eq!(
        attach.destinations(),
        ["other"],
        "the file of the directory switched to"
    );
    assert!(remotes_banners(&mut attach).is_empty(), "a switch never reports");
}

// MARK: - the flow

#[test]
fn no_remotes_does_nothing() {
    let mut attach = Attach::new(None);
    let window = attach.fixture.window;
    attach.core().attach_remote(window);
    attach.idle();
    assert!(attach.pending().is_none());
    assert!(attach.runner.hosts().is_empty());
    assert_eq!(attach.fixture.core.remote_wake(), None);
}

#[test]
fn a_single_remote_is_listed_without_asking() {
    let mut attach = Attach::new(Some("studio Mac Studio\n"));
    let window = attach.fixture.window;
    attach.core().attach_remote(window);
    attach.wait_for_runs(1);
    assert_eq!(attach.runner.hosts(), ["studio"]);
    assert!(attach.pending().is_none());
    assert_eq!(attach.hud(), Some(progress("listing sessions on studio…")));
    attach.runner.answer(0, listing(&[]));
    attach.drive("the session picker opens", |attach| attach.pending().is_some());
    let picker = attach.pending().unwrap();
    attach.cancel(&picker);
    attach.drive("the flow ends", |attach| attach.fixture.core.remote_wake().is_none());
}

#[test]
fn dismissing_the_machine_picker_contacts_nothing() {
    let mut attach = Attach::new(Some("studio\nmini\n"));
    let window = attach.fixture.window;
    attach.core().attach_remote(window);
    let hosts = attach.pending().expect("the machine picker opens");
    assert_eq!(hosts.prompt.as_deref(), Some("Attach from which Mac?"));
    assert_eq!(hosts.items[1].label, "mini");
    assert_eq!(hosts.items[1].subtitle, None, "a bare destination has no subtitle");
    attach.cancel(&hosts);
    attach.idle();
    assert!(attach.pending().is_none());
    assert!(attach.runner.hosts().is_empty());
    assert_eq!(attach.hud(), None);
    assert_eq!(attach.fixture.core.remote_wake(), None, "the flow ended");
}

#[test]
fn the_progress_panel_is_gone_before_the_session_picker_and_the_attach_closes_its_own() {
    let mut attach = Attach::new(Some("studio\n"));
    let window = attach.fixture.window;
    attach.core().attach_remote(window);
    attach.wait_for_runs(1);
    attach.runner.answer(0, listing(&[build_session()]));
    attach.drive("the session picker opens", |attach| attach.pending().is_some());
    assert_eq!(attach.hud(), None, "the progress panel is gone before the picker shows");
    let sessions = attach.pending().unwrap();
    assert_eq!(sessions.prompt.as_deref(), Some("Attach from studio"));
    assert_eq!(
        sessions.items,
        [
            ngterm_proto::PickItem {
                id: REMOTE_SESSION.to_owned(),
                label: "build".to_owned(),
                subtitle: Some("main/work  ·  release prep  ·  /repo  ·  vim".to_owned()),
                status: Some("blocked".to_owned()),
                status_color: Some("#ff8800".to_owned()),
                status_shape: Some("star".to_owned()),
            },
            new_session_row()
        ],
        "the row carries the far session's status for its glyph"
    );
    attach.resolve(&sessions, sessions.picked(0).unwrap());
    attach.wait_for_runs(2);
    assert_eq!(attach.hud(), Some(progress("attaching build…")));
    assert!(attach.pending().is_none());
    let before = attach.fixture.store().sessions().count();
    attach.runner.answer(1, listing(&[build_session()]));
    attach.drive("the attach lands", |attach| {
        attach.fixture.store().sessions().count() == before + 1
    });
    assert_eq!(attach.hud(), None, "the attach panel closes");
}

#[test]
fn a_failed_listing_shows_a_timed_error_panel_and_no_picker() {
    let mut attach = Attach::new(Some("studio\n"));
    let window = attach.fixture.window;
    attach.core().attach_remote(window);
    attach.wait_for_runs(1);
    attach
        .runner
        .answer(0, refused("ssh: \u{1b}[31mconnection refused\u{1b}[0m\n"));
    attach.drive("the error panel shows", |attach| {
        attach.hud().is_some_and(|hud| hud.spinner.is_none())
    });
    assert_eq!(attach.hud(), Some(failure("studio: ssh: connection refused")));
    assert!(attach.pending().is_none());
    assert_eq!(attach.fixture.core.remote_wake(), None, "the flow ended");
}

#[test]
fn a_refused_attach_shows_a_timed_error_panel() {
    let mut attach = Attach::new(Some("studio\n"));
    let window = attach.fixture.window;
    attach.core().attach_remote(window);
    attach.wait_for_runs(1);
    attach.runner.answer(0, listing(&[build_session()]));
    attach.drive("the session picker opens", |attach| attach.pending().is_some());
    let sessions = attach.pending().unwrap();
    attach.resolve(&sessions, sessions.picked(0).unwrap());
    attach.wait_for_runs(2);
    // the session went away between the listing and the attach
    attach.runner.answer(1, listing(&[]));
    attach.drive("the error panel shows", |attach| {
        attach.hud().is_some_and(|hud| hud.spinner.is_none())
    });
    assert_eq!(
        attach.hud(),
        Some(failure(&format!("no attachable session {REMOTE_SESSION} on studio")))
    );
}

#[test]
fn an_attach_leaves_a_hud_another_caller_posted_during_its_wait_alone() {
    let mut attach = Attach::new(Some("studio\n"));
    let window = attach.fixture.window;
    attach.core().attach_remote(window);
    attach.wait_for_runs(1);
    let target = wire(attach.session);
    let response = attach.core().dispatch(&Request {
        cmd: Command::Session(SessionCmd::HudOpen),
        target: Some(target),
        args: Some(args(|a| a.message = Some("an agent's own panel".to_owned()))),
    });
    assert!(response.ok, "{response:?}");
    attach.runner.answer(0, listing(&[build_session()]));
    attach.drive("the session picker opens", |attach| attach.pending().is_some());
    assert_eq!(
        attach.hud().map(|hud| hud.message).as_deref(),
        Some("an agent's own panel"),
        "the finished listing closed nothing"
    );
    let sessions = attach.pending().unwrap();
    attach.cancel(&sessions);
}

#[test]
fn an_overlong_error_is_cut_to_what_a_hud_accepts() {
    let mut attach = Attach::new(Some("studio\n"));
    let window = attach.fixture.window;
    attach.core().attach_remote(window);
    attach.wait_for_runs(1);
    attach.runner.answer(0, refused(&"q\u{301}".repeat(400)));
    attach.drive("the error panel shows", |attach| {
        attach.hud().is_some_and(|hud| hud.spinner.is_none())
    });
    let message = attach.hud().unwrap().message;
    assert!(crate::hud::layout::text_length(&message) <= HudSpec::MAX_TEXT_LENGTH);
    // a reason this long leaves no room for the name, as upstream's `CommandFailure.message` cuts it
    assert!(message.starts_with("studio: q\u{301}"), "{message}");
}

#[test]
fn dismissing_the_session_picker_attaches_nothing() {
    let mut attach = Attach::new(Some("studio\n"));
    let window = attach.fixture.window;
    attach.core().attach_remote(window);
    attach.wait_for_runs(1);
    attach.runner.answer(0, listing(&[build_session()]));
    attach.drive("the session picker opens", |attach| attach.pending().is_some());
    let before = attach.fixture.store().sessions().count();
    let sessions = attach.pending().unwrap();
    attach.cancel(&sessions);
    attach.idle();
    assert!(attach.pending().is_none());
    assert_eq!(attach.runner.hosts().len(), 1, "nothing is attached");
    assert_eq!(attach.fixture.store().sessions().count(), before);
    assert_eq!(attach.fixture.core.remote_wake(), None);
}

#[test]
fn a_window_closed_during_the_listing_gets_no_picker() {
    let mut attach = Attach::new(Some("studio\n"));
    let window = attach.fixture.window;
    let other = attach.core().library_mut().new_window(None);
    attach.core().attach_remote(window);
    attach.wait_for_runs(1);
    attach.core().close_window(window);
    attach.runner.answer(0, listing(&[build_session()]));
    attach.drive("the flow ends", |attach| attach.fixture.core.remote_wake().is_none());
    assert!(
        attach
            .fixture
            .core
            .modal_slot(window)
            .and_then(|slot| slot.pending())
            .is_none()
    );
    assert!(
        attach
            .fixture
            .core
            .modal_slot(other)
            .and_then(|slot| slot.pending())
            .is_none()
    );
    assert_eq!(attach.runner.hosts().len(), 1, "nothing is attached");
}

#[test]
fn attach_does_nothing_while_a_modal_holds_the_window() {
    let mut attach = Attach::new(Some("studio\n"));
    let window = attach.fixture.window;
    let busy = PendingPick {
        id: "busy".to_owned(),
        items: vec![ngterm_proto::PickItem {
            id: "a".to_owned(),
            label: "A".to_owned(),
            subtitle: None,
            ..ngterm_proto::PickItem::default()
        }],
        prompt: None,
        query: None,
        allow_custom: false,
        selection: None,
    };
    attach.core().modals.open_pick(window, busy).unwrap();
    attach.core().attach_remote(window);
    attach.idle();
    assert!(attach.runner.hosts().is_empty());
    assert_eq!(attach.pending().map(|pick| pick.id).as_deref(), Some("busy"));
    assert_eq!(attach.hud(), None);
}

#[test]
fn a_window_switch_while_choosing_the_mac_keeps_the_attach_in_the_starting_window() {
    let mut attach = Attach::new(Some("studio\nmini\n"));
    let window = attach.fixture.window;
    let other = attach.core().library_mut().new_window(None);
    attach.core().report_frontmost(window);
    attach.core().attach_remote(window);
    let hosts = attach.pending().expect("the machine picker opens");
    attach.core().report_frontmost(other);
    attach.resolve(&hosts, hosts.picked(0).unwrap());
    attach.wait_for_runs(1);
    assert_eq!(attach.hud(), Some(progress("listing sessions on studio…")));
    attach.runner.answer(0, listing(&[build_session()]));
    attach.drive("the session picker opens in the starting window", |attach| {
        attach.pending().is_some()
    });
    assert!(
        attach
            .fixture
            .core
            .modal_slot(other)
            .and_then(|slot| slot.pending())
            .is_none()
    );
    let sessions = attach.pending().unwrap();
    attach.resolve(&sessions, sessions.picked(0).unwrap());
    attach.wait_for_runs(2);
    let before = attach.fixture.store().sessions().count();
    attach.runner.answer(1, listing(&[build_session()]));
    attach.drive("the attach lands in the starting window", |attach| {
        attach.fixture.store().sessions().count() == before + 1
    });
    let attached = attach.fixture.store().sessions().last().unwrap();
    assert_eq!(attached.remote_host(), Some("studio"));
}

// MARK: - Attach All

/// One pane session named `name`, its id and daemon from `number`, with the far side's `parent` when it has one.
fn numbered_session(name: &str, number: u128, parent: Option<u128>) -> serde_json::Value {
    let id = Uuid::from_u128(number).to_string().to_uppercase();
    let daemon = crate::live::daemon_name(Uuid::from_u128(0x1000 + number));
    let mut session = json!({
        "id": id, "name": name, "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
        "workspaceName": "work", "cwd": "/repo", "panes": [{"pane": "left", "daemon": daemon}],
    });
    if let Some(parent) = parent {
        session["parent"] = json!(Uuid::from_u128(parent).to_string().to_uppercase());
    }
    session
}

/// A far page companion, which has no panes.
fn page_session(number: u128, parent: u128) -> serde_json::Value {
    json!({
        "id": Uuid::from_u128(number).to_string().to_uppercase(), "name": "page", "windowID": "w1",
        "windowName": "main", "workspaceID": "ws1", "workspaceName": "work", "cwd": "/repo", "panes": [],
        "parent": Uuid::from_u128(parent).to_string().to_uppercase(), "content": "page",
    })
}

impl Attach {
    /// Opens the session picker on `sessions`, answering the `call`th remote command.
    fn open_session_picker(&mut self, call: usize, sessions: &[serde_json::Value]) -> PendingPick {
        let window = self.fixture.window;
        self.core().attach_remote(window);
        self.wait_for_runs(call + 1);
        self.runner.answer(call, listing(sessions));
        self.drive("the session picker opens", |attach| attach.pending().is_some());
        self.pending().unwrap()
    }

    /// Picks the picker's Attach All row, then answers the listing it asks for.
    fn attach_all(&mut self, call: usize, picker: &PendingPick, sessions: &[serde_json::Value]) {
        let row = picker.items.iter().position(|item| item.label == "Attach All").unwrap();
        self.resolve(picker, picker.picked(row).unwrap());
        self.wait_for_runs(call + 1);
        assert_eq!(self.hud(), Some(progress("attaching all sessions…")));
        self.runner.answer(call, listing(sessions));
        self.drive("the attach ends", |attach| {
            attach.hud() != Some(progress("attaching all sessions…"))
        });
    }

    fn remote_rows(&self) -> Vec<String> {
        self.fixture
            .store()
            .sessions()
            .filter(|row| row.remote_host().is_some())
            .map(crate::model::Session::display_name)
            .collect()
    }
}

#[test]
fn a_single_session_offers_no_attach_all() {
    let mut attach = Attach::new(Some("studio\n"));
    let picker = attach.open_session_picker(0, &[build_session()]);
    assert_eq!(picker.items.len(), 2);
    assert_eq!(picker.items[1], new_session_row());
    assert_eq!(picker.selection, None);
}

#[test]
fn attach_all_follows_the_sessions_so_enter_on_its_first_row_still_attaches_one() {
    let mut attach = Attach::new(Some("studio\n"));
    let sessions = [numbered_session("one", 1, None), numbered_session("two", 2, None)];
    let picker = attach.open_session_picker(0, &sessions);
    assert_eq!(picker.items[0].label, "one");
    assert_eq!(picker.selection, None);
    let all = &picker.items[2];
    assert_eq!(all.id, "attach-all");
    assert_eq!(all.label, "Attach All");
    assert_eq!(all.subtitle.as_deref(), Some("2 sessions not attached here"));
}

#[test]
fn attach_all_attaches_each_session_once_with_companions_under_their_parents() {
    let mut attach = Attach::new(Some("studio\n"));
    let listed = [
        // the companion comes first, as nothing orders the far side's list
        numbered_session("logs", 3, Some(1)),
        numbered_session("one", 1, None),
        numbered_session("two", 2, None),
        page_session(4, 1),
    ];
    let picker = attach.open_session_picker(0, &listed);
    assert_eq!(
        picker.items.len(),
        5,
        "three sessions, Attach All and New Session; the page stays out of the picker: {:?}",
        picker.items
    );
    attach.attach_all(1, &picker, &listed);
    assert_eq!(attach.hud(), None, "the panel closes");
    assert_eq!(attach.remote_rows(), ["one", "logs", "two"]);
    let store = attach.fixture.store();
    let row = |name: &str| store.sessions().find(|row| row.display_name() == name).unwrap();
    assert_eq!(row("logs").parent_id(), Some(row("one").id()));
    assert_eq!(
        store.selected_session_id(),
        Some(row("one").id()),
        "the first row is selected"
    );
}

#[test]
fn attach_all_skips_what_is_already_attached_and_attaches_what_appeared_since_the_list() {
    let mut attach = Attach::new(Some("studio\n"));
    let first = [numbered_session("one", 1, None), numbered_session("two", 2, None)];
    let picker = attach.open_session_picker(0, &first);
    attach.resolve(&picker, picker.picked(0).unwrap());
    attach.wait_for_runs(2);
    attach.runner.answer(1, listing(&first));
    attach.drive("the first session attaches", |attach| attach.remote_rows().len() == 1);
    // the attach selected its row, which the next flow's panel goes over
    attach.session = attach.fixture.store().selected_session_id().unwrap();

    let second = [
        numbered_session("one", 1, None),
        numbered_session("two", 2, None),
        numbered_session("three", 3, None),
    ];
    let picker = attach.open_session_picker(2, &second);
    assert_eq!(
        picker.items[picker.items.len() - 2].subtitle.as_deref(),
        Some("2 sessions not attached here")
    );
    let fresh = [
        numbered_session("one", 1, None),
        numbered_session("three", 3, None),
        numbered_session("four", 4, None),
    ];
    attach.attach_all(3, &picker, &fresh);
    assert_eq!(
        attach.remote_rows(),
        ["one", "three", "four"],
        "two went since the list"
    );

    let picker = attach.open_session_picker(
        4,
        &[numbered_session("one", 1, None), numbered_session("five", 5, None)],
    );
    assert_eq!(picker.items.len(), 3, "one session left to attach offers no Attach All");
    assert_eq!(picker.items[2], new_session_row());
}

#[test]
fn attach_all_with_sessions_it_cannot_address_attaches_the_rest_and_says_so_over_the_selected_row() {
    let settings = AppSettings {
        notifications_enabled: Some(false),
        ..AppSettings::default()
    };
    let mut attach = Attach::with(Some("studio\n"), settings);
    let broken = |name: &str, number: u128| {
        let mut session = numbered_session(name, number, None);
        session["panes"] = json!([]);
        session
    };
    let listed = [numbered_session("one", 1, None), broken("bad", 2), broken("worse", 3)];
    let picker = attach.open_session_picker(0, &listed);
    attach.attach_all(1, &picker, &listed);
    assert_eq!(attach.remote_rows(), ["one"]);
    assert_eq!(attach.hud(), None, "the progress panel closes");
    attach.session = attach.fixture.store().selected_session_id().unwrap();
    let reason = "studio reported panes ngterm cannot address";
    assert_eq!(
        attach.hud(),
        Some(failure(&format!(
            "attached 1 of 3 from studio; bad: {reason}; worse: {reason}"
        ))),
        "the row in view says so, whatever the banners setting holds"
    );
}

#[test]
fn attach_all_attaching_nothing_says_why_over_the_starting_session() {
    let mut attach = Attach::new(Some("studio\n"));
    let mut broken = numbered_session("broken", 2, None);
    broken["panes"] = json!([]);
    let picker = attach.open_session_picker(0, &[numbered_session("one", 1, None), broken.clone()]);
    attach.attach_all(1, &picker, &[broken]);
    assert_eq!(attach.remote_rows(), Vec::<String>::new());
    assert_eq!(
        attach.hud(),
        Some(failure(
            "attached 0 of 1 from studio; broken: studio reported panes ngterm cannot address"
        ))
    );
}

#[test]
fn attach_all_finding_nothing_left_says_so() {
    let mut attach = Attach::new(Some("studio\n"));
    let picker = attach.open_session_picker(0, &[numbered_session("one", 1, None), numbered_session("two", 2, None)]);
    attach.attach_all(1, &picker, &[]);
    assert_eq!(attach.remote_rows(), Vec::<String>::new());
    assert_eq!(attach.hud(), Some(failure("nothing to attach on studio")));
}

// MARK: - sessions made and ended on the machine

const CREATED: &str = "0E2B7C1A-5D4F-4E3B-9A61-7C2D8F0B1A23";

fn new_session_row() -> ngterm_proto::PickItem {
    ngterm_proto::PickItem {
        id: "new-session".to_owned(),
        label: "New Session".to_owned(),
        subtitle: Some("create a session on studio and attach it".to_owned()),
        ..ngterm_proto::PickItem::default()
    }
}

fn reply(body: &serde_json::Value) -> RemoteCommandResult {
    RemoteCommandResult {
        status: 0,
        stdout: body.to_string(),
        stderr: String::new(),
    }
}

fn restore_mode(active: &str) -> RemoteCommandResult {
    reply(&json!({"ok": true, "result": {"restore": {
        "configured": active, "requestedAtLaunch": active, "active": active, "restartRequired": false,
    }}}))
}

fn created_receipt() -> RemoteCommandResult {
    reply(&json!({"ok": true, "result": {"id": CREATED}}))
}

/// The session the origin created, as its listing shows it.
fn created_session() -> serde_json::Value {
    let daemon = crate::live::daemon_name(Uuid::from_u128(0xc1));
    json!({
        "id": CREATED, "name": "fresh", "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
        "workspaceName": "work", "cwd": "/home", "panes": [{"pane": "left", "daemon": daemon}],
    })
}

fn panel(action: &str, text: &str, progress: bool) -> HudSpec {
    if progress {
        return HudSpec {
            spinner: Some(HudSpinner::Bar),
            hide_after: Some(30.0),
            ..HudSpec::new(format!("{action}: {text}"))
        };
    }
    HudSpec {
        hide_after: Some(5.0),
        ..HudSpec::new(format!("{action}: {text}"))
    }
}

impl Attach {
    /// Picks the picker's New Session row, which asks the origin for its restore mode as the `call`th remote command.
    fn pick_new_session(&mut self, call: usize, picker: &PendingPick) {
        let row = picker.items.iter().position(|item| item.id == "new-session").unwrap();
        self.resolve(picker, picker.picked(row).unwrap());
        self.wait_for_runs(call + 1);
    }

    /// [`Attach::pick_new_session`], the origin saying it runs live sessions, so it is asked to create as the next call.
    fn pick_new_session_on_live(&mut self, call: usize, picker: &PendingPick) {
        self.pick_new_session(call, picker);
        self.runner.answer(call, restore_mode("live"));
        self.wait_for_runs(call + 2);
    }

    /// One attached row, selected, and the remote command count it took.
    fn attached_row(&mut self) -> Uuid {
        let picker = self.open_session_picker(0, &[build_session()]);
        self.resolve(&picker, picker.picked(0).unwrap());
        self.wait_for_runs(2);
        self.runner.answer(1, listing(&[build_session()]));
        self.drive("the attach lands", |attach| attach.remote_rows() == ["build"]);
        self.fixture.store().selected_session_id().unwrap()
    }

    fn act(&mut self, action: SidebarAction) -> bool {
        let window = self.fixture.window;
        self.core().sidebar_action(window, action)
    }
}

#[test]
fn a_machine_with_nothing_to_attach_offers_only_new_session() {
    let mut attach = Attach::new(Some("studio\n"));
    let picker = attach.open_session_picker(0, &[]);
    assert_eq!(picker.items, [new_session_row()]);
    assert_eq!(attach.hud(), None);
}

#[test]
fn a_picker_full_of_sessions_keeps_them_and_one_over_is_refused() {
    let many = |count: usize| -> Vec<serde_json::Value> {
        (1..=count)
            .map(|number| numbered_session("one", number as u128, None))
            .collect()
    };
    let mut attach = Attach::new(Some("studio\n"));
    let picker = attach.open_session_picker(0, &many(crate::modal::MAX_PICK_ITEMS));
    assert_eq!(picker.items.len(), crate::modal::MAX_PICK_ITEMS);
    assert!(
        picker.items.iter().all(|item| item.label == "one"),
        "no room for another row"
    );

    let mut attach = Attach::new(Some("studio\n"));
    let window = attach.fixture.window;
    attach.core().attach_remote(window);
    attach.wait_for_runs(1);
    attach
        .runner
        .answer(0, listing(&many(crate::modal::MAX_PICK_ITEMS + 1)));
    attach.drive("the error panel shows", |attach| {
        attach.hud().is_some_and(|hud| hud.spinner.is_none())
    });
    assert_eq!(attach.hud(), Some(failure("studio offers too many sessions")));
}

#[test]
fn new_session_creates_it_on_the_machine_and_attaches_it_selected() {
    let mut attach = Attach::new(Some("studio\n"));
    let picker = attach.open_session_picker(0, &[build_session()]);
    attach.pick_new_session(1, &picker);
    assert_eq!(
        attach.hud(),
        Some(panel("New Session", "creating a session on studio…", true))
    );
    let check = attach.runner.lines()[1].clone();
    assert!(check.contains("restore") && check.contains("mode"), "{check}");
    attach.runner.answer(1, restore_mode("live"));
    attach.wait_for_runs(3);
    let line = attach.runner.lines()[2].clone();
    assert!(
        line.contains("session") && line.contains("new") && line.contains("--no-select"),
        "{line}"
    );
    assert!(line.contains("--json"), "{line}");
    assert!(
        attach.remote_rows().is_empty(),
        "nothing attaches before the origin answers"
    );
    attach.runner.answer(2, created_receipt());
    attach.wait_for_runs(4);
    assert_eq!(attach.hud(), Some(panel("New Session", "attaching the session…", true)));
    attach.runner.answer(3, listing(&[build_session(), created_session()]));
    attach.drive("the row lands", |attach| attach.remote_rows() == ["fresh"]);
    let selected = attach.fixture.store().active_session().unwrap();
    assert_eq!(selected.display_name(), "fresh");
    assert_eq!(selected.remote_host(), Some("studio"));
    assert_eq!(attach.hud(), None, "the panel closes");
    assert_eq!(attach.runner.lines().len(), 4, "the origin is asked once");
}

#[test]
fn new_session_on_a_machine_outside_live_sessions_creates_nothing_and_says_why() {
    let answers = [
        (
            restore_mode("none"),
            "studio does not run live sessions (restore mode none), so a session made there could not attach; \
             nothing was created",
        ),
        (
            RemoteCommandResult {
                status: 64,
                stdout: String::new(),
                stderr: "Error: Unknown option\n".to_owned(),
            },
            "the ngterm on studio is too old for this; nothing was created",
        ),
        (
            reply(&json!({"ok": false, "error": "ngterm is stopped"})),
            "ngterm is stopped; nothing was created",
        ),
        (
            refused("ssh: connection lost\n"),
            "studio gave no answer (ssh: connection lost); nothing was created",
        ),
        (
            reply(&json!({"ok": true, "result": {}})),
            "studio did not name its restore mode; nothing was created",
        ),
    ];
    for (answer, shown) in answers {
        let mut attach = Attach::new(Some("studio\n"));
        let picker = attach.open_session_picker(0, &[]);
        attach.pick_new_session(1, &picker);
        attach.runner.answer(1, answer);
        attach.drive("the error panel shows", |attach| {
            attach.hud().is_some_and(|hud| hud.spinner.is_none())
        });
        assert_eq!(attach.hud(), Some(panel("New Session", shown, false)));
        assert_eq!(attach.runner.lines().len(), 2, "the origin is never asked to create");
        assert_eq!(attach.fixture.core.remote_wake(), None, "the flow ended");
    }
}

#[test]
fn a_created_session_not_listed_yet_is_listed_again_and_never_created_twice() {
    let mut attach = Attach::new(Some("studio\n"));
    let picker = attach.open_session_picker(0, &[]);
    attach.pick_new_session_on_live(1, &picker);
    attach.runner.answer(2, created_receipt());
    attach.wait_for_runs(4);
    attach.runner.answer(3, listing(&[]));
    attach.wait_for_runs(5);
    assert!(attach.remote_rows().is_empty());
    assert!(attach.runner.lines()[4].contains("tree"), "the second call only lists");
    attach.runner.answer(4, listing(&[created_session()]));
    attach.drive("the row lands", |attach| attach.remote_rows() == ["fresh"]);
}

#[test]
fn a_session_never_attachable_names_what_attaches_it() {
    let mut attach = Attach::new(Some("studio\n"));
    let start = Instant::now();
    let elapsed = Arc::new(Mutex::new(Duration::ZERO));
    let reading = Arc::clone(&elapsed);
    attach.core().remote.clock = Box::new(move || start + *reading.lock().unwrap());
    let picker = attach.open_session_picker(0, &[]);
    attach.pick_new_session_on_live(1, &picker);
    attach.runner.answer(2, created_receipt());
    attach.wait_for_runs(4);
    attach.runner.answer(3, listing(&[]));
    // past the whole budget on every tick, so a listing however late is past its pause and the deadline ends the wait
    attach.drive("the wait ends", |attach| {
        *elapsed.lock().unwrap() += Duration::from_secs(21);
        attach.hud().is_some_and(|hud| hud.spinner.is_none())
    });
    assert_eq!(
        attach.hud(),
        Some(panel(
            "New Session",
            &format!(
                "created session {CREATED} on studio but could not attach it: it did not become attachable; \
                 `ngtermctl live attach studio {CREATED}` attaches it"
            ),
            false
        ))
    );
    assert_eq!(attach.fixture.core.remote_wake(), None, "the flow ended");
}

#[test]
fn a_create_the_origin_refuses_or_never_answers_says_so_and_attaches_nothing() {
    let answers = [
        (
            reply(&json!({"ok": false, "error": "no window to create in"})),
            "no window to create in".to_owned(),
        ),
        (
            refused("ssh: connection lost\n"),
            "studio gave no answer (ssh: connection lost); whether the session was created there is unknown".to_owned(),
        ),
        (
            RemoteCommandResult {
                status: 64,
                stdout: String::new(),
                stderr: "Error: Unknown option '--no-select'\n".to_owned(),
            },
            "the ngterm on studio is too old for this; nothing was created".to_owned(),
        ),
    ];
    for (answer, shown) in answers {
        let mut attach = Attach::new(Some("studio\n"));
        let picker = attach.open_session_picker(0, &[]);
        attach.pick_new_session_on_live(1, &picker);
        attach.runner.answer(2, answer);
        attach.drive("the error panel shows", |attach| {
            attach.hud().is_some_and(|hud| hud.spinner.is_none())
        });
        assert_eq!(attach.hud(), Some(panel("New Session", &shown, false)));
        assert!(attach.remote_rows().is_empty());
        assert_eq!(attach.runner.lines().len(), 3, "never retried");
    }
}

#[test]
fn new_session_from_a_remote_row_lands_in_that_rows_workspace() {
    let mut attach = Attach::new(Some("studio\n"));
    let row = attach.attached_row();
    let home = attach.fixture.store().current_workspace_id().unwrap();
    let store = attach.fixture.store_mut().unwrap();
    let other = store.add_workspace("other");
    store.select_workspace(other);
    attach.session = row;
    assert!(attach.act(SidebarAction::NewRemoteSession(row)));
    attach.wait_for_runs(3);
    attach.runner.answer(2, restore_mode("live"));
    attach.wait_for_runs(4);
    attach.runner.answer(3, created_receipt());
    attach.wait_for_runs(5);
    attach.runner.answer(4, listing(&[build_session(), created_session()]));
    attach.drive("the row lands", |attach| attach.remote_rows() == ["build", "fresh"]);
    let names: Vec<String> = attach
        .fixture
        .store()
        .workspace(home)
        .unwrap()
        .sessions
        .iter()
        .map(crate::model::Session::display_name)
        .collect();
    assert!(names.contains(&"fresh".to_owned()), "{names:?}");
}

#[test]
fn new_session_on_a_local_row_does_nothing() {
    let mut attach = Attach::new(Some("studio\n"));
    let local = attach.session;
    assert!(!attach.act(SidebarAction::NewRemoteSession(local)));
    assert!(!attach.act(SidebarAction::EndRemoteSession(local)));
    assert!(attach.runner.lines().is_empty());
    assert!(attach.fixture.store().session(local).is_some());
}

#[test]
fn end_session_closes_it_on_the_machine_then_the_row_here() {
    let mut attach = Attach::new(Some("studio\n"));
    let row = attach.attached_row();
    attach.session = row;
    assert!(attach.act(SidebarAction::EndRemoteSession(row)));
    attach.wait_for_runs(3);
    assert_eq!(
        attach.hud(),
        Some(panel("End Session", "ending the session on studio…", true))
    );
    let line = attach.runner.lines()[2].clone();
    assert!(
        line.contains("close") && line.contains(REMOTE_SESSION) && line.contains("--json"),
        "{line}"
    );
    assert!(!line.contains("undoable"), "ended for good: {line}");
    assert_eq!(attach.remote_rows(), ["build"], "the row waits for the origin");
    attach
        .runner
        .answer(2, reply(&json!({"ok": true, "result": {"id": REMOTE_SESSION}})));
    attach.drive("the row closes", |attach| attach.remote_rows().is_empty());
    assert!(attach.fixture.store().session(row).is_none());
    assert_eq!(attach.fixture.core.remote_wake(), None, "nothing remote is left to run");
}

#[test]
fn end_session_the_origin_refuses_or_never_answers_keeps_the_row() {
    let answers = [
        (
            reply(&json!({"ok": false, "error": "no session matches"})),
            "no session matches".to_owned(),
        ),
        (
            refused("ssh: connection lost\n"),
            "studio gave no answer (ssh: connection lost); whether the session ended there is unknown".to_owned(),
        ),
    ];
    for (answer, shown) in answers {
        let mut attach = Attach::new(Some("studio\n"));
        let row = attach.attached_row();
        attach.session = row;
        assert!(attach.act(SidebarAction::EndRemoteSession(row)));
        attach.wait_for_runs(3);
        attach.runner.answer(2, answer);
        attach.drive("the error panel shows", |attach| {
            attach.hud().is_some_and(|hud| hud.spinner.is_none())
        });
        assert_eq!(attach.hud(), Some(panel("End Session", &shown, false)));
        assert_eq!(attach.remote_rows(), ["build"]);
    }
}

#[test]
fn a_second_end_session_while_one_runs_asks_nothing_more() {
    let mut attach = Attach::new(Some("studio\n"));
    let row = attach.attached_row();
    assert!(attach.act(SidebarAction::EndRemoteSession(row)));
    assert!(attach.act(SidebarAction::EndRemoteSession(row)));
    attach.wait_for_runs(3);
    attach.idle();
    assert_eq!(attach.runner.lines().len(), 3);
}

#[test]
fn a_row_closed_while_its_end_runs_leaves_nothing_to_do() {
    let mut attach = Attach::new(Some("studio\n"));
    let row = attach.attached_row();
    assert!(attach.act(SidebarAction::EndRemoteSession(row)));
    attach.wait_for_runs(3);
    attach.fixture.store_mut().unwrap().close_session(row);
    attach
        .runner
        .answer(2, reply(&json!({"ok": true, "result": {"id": REMOTE_SESSION}})));
    attach.idle();
    assert_eq!(attach.fixture.core.remote_wake(), None, "the run ended");
    assert!(attach.banners().is_empty());
}

#[test]
fn a_row_soft_closed_while_its_end_runs_cannot_be_undone_once_the_session_ended() {
    let mut attach = Attach::new(Some("studio\n"));
    let row = attach.attached_row();
    assert!(attach.act(SidebarAction::EndRemoteSession(row)));
    attach.wait_for_runs(3);
    assert!(attach.fixture.store_mut().unwrap().soft_close_session(row));
    attach
        .runner
        .answer(2, reply(&json!({"ok": true, "result": {"id": REMOTE_SESSION}})));
    attach.drive("the run ends", |attach| attach.fixture.core.remote_wake().is_none());
    let store = attach.fixture.store_mut().unwrap();
    assert!(!store.undo_pending_close(None), "nothing is left to undo");
    assert!(store.session(row).is_none());
}
