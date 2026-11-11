use crate::{ClipboardReadRequest, ReadScope};
use std::ffi::OsString;
use std::path::Path;
use wire::DelegatedRequest;

use ngterm_session::cli::{self, Command, default_argv, variable as lookup};
use ngterm_session::server::{EXIT_FAILED, EXIT_USAGE};

use super::*;

impl Live {
    /// The single metadata message actually posted by a pane's setter, with no session connected to drain it.
    pub(crate) fn take_posted_metadata(&self) -> ReplyMetadata {
        let mut outbox = lock(&self.outbox);
        assert_eq!(outbox.queue.len(), 1, "{:?}", outbox.queue);
        let Some(Message::Metadata(metadata)) = outbox.queue.pop_front() else {
            panic!("not metadata");
        };
        metadata
    }

    pub(crate) fn take_posted_reload(&self) -> ReplyMetadata {
        let mut outbox = lock(&self.outbox);
        assert_eq!(outbox.queue.len(), 1, "{:?}", outbox.queue);
        let Some(Message::ConfigReload(metadata)) = outbox.queue.pop_front() else {
            panic!("not a config reload");
        };
        metadata
    }
}

#[test]
fn identical_config_reloads_are_posted_with_final_facts_and_remembered_for_reconnect() {
    let live = Live::new(ReadPause::new(), 1024, true);
    let metadata = ReplyMetadata {
        color_scheme: Some(wire::ColorScheme::Dark),
        ..ReplyMetadata::default()
    };
    for _ in 0..2 {
        live.config_reloaded(metadata.clone());
        let mut outbox = lock(&live.outbox);
        assert_eq!(outbox.queue.len(), 1);
        let Some(Message::ConfigReload(got)) = outbox.queue.pop_front() else {
            panic!("not a reload");
        };
        assert_eq!(got, metadata);
        assert_eq!(*lock(&live.metadata), metadata);
    }
    live.update_metadata(metadata);
    assert!(lock(&live.outbox).queue.is_empty(), "ordinary facts still deduplicate");
}

fn attach(argv: &[&str]) -> LiveAttach {
    LiveAttach {
        executable: "/opt/ngterm/bin/ngterm-session".to_owned(),
        session: "ngterm-0123".to_owned(),
        argv: argv.iter().map(|arg| (*arg).to_owned()).collect(),
    }
}

/// The app's environment: an outer session's variables, which the launch overrides.
fn base() -> Vec<(String, String)> {
    [
        ("PATH", "/usr/bin"),
        ("NGTERM_LIVE_SESSION", "outer"),
        ("NGTERM_LIVE_MANAGED", "stale"),
        ("GHOSTTY_LOG", "1"),
    ]
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .to_vec()
}

/// A wrapped pane's prepared command: the wrap configuration's variables, as `Host::prepare` sets them.
fn command() -> PaneCommand {
    let dir = std::env::temp_dir().join("ngterm-live-test");
    PaneCommand::new("/bin/sh")
        .cwd(std::env::temp_dir())
        .env("NGTERM_LIVE_DIR", &dir)
        .env("NGTERM_LIVE_SESSION", "")
        .env("NGTERM_LIVE_SESSION_PREFIX", "")
        .env("NGTERM_LIVE_NO_DETACH_KEY", "1")
        .env("NGTERM_LIVE_MANAGED", "n1")
        .env("NGTERM_LIVE_MANAGED_CLAIM", "1")
        .env("SHELL", "/bin/zsh")
        .env_remove("GHOSTTY_LOG")
}

#[test]
fn the_in_process_attach_is_the_one_its_shell_line_runs() {
    for argv in [&[][..], &["/bin/zsh", "-lic", "top"][..]] {
        let attach = attach(argv);
        let command = command();
        let session = resolve(&attach, &command, base(), None).unwrap();
        // the CLI reads its command line…
        let arguments: Vec<OsString> = attach.arguments()[1..].iter().map(OsString::from).collect();
        let Ok(Command::Attach { name, argv: line_argv }) = cli::parse(&arguments) else {
            panic!("not an attach: {arguments:?}");
        };
        // …and its environment, the child's
        let contract = Contract::from_env(|key| lookup(&session.spec.env, key).map(Into::into));
        assert_eq!(session.name, contract.session_name(&name).unwrap());
        assert_eq!(Some(session.directory.clone()), contract.directory);
        assert_eq!(session.managed, contract.managed);
        assert_eq!(
            session.managed,
            Some(Managed {
                nonce: "n1".to_owned(),
                claim: true,
            })
        );
        let expected = if line_argv.is_empty() {
            default_argv(Some(std::ffi::OsStr::new("/bin/zsh")))
        } else {
            line_argv
        };
        assert_eq!(session.spec.argv, expected);
        assert_eq!(session.executable, PathBuf::from(&attach.executable));
        // the session is created in the directory and environment the CLI would have run in
        assert_eq!(
            session.spec.cwd.as_deref(),
            Some(std::env::temp_dir().to_string_lossy().as_ref())
        );
        assert_eq!(lookup(&session.spec.env, "PATH"), Some("/usr/bin"));
        assert_eq!(lookup(&session.spec.env, "NGTERM_LIVE_SESSION"), Some(""));
        assert_eq!(lookup(&session.spec.env, "GHOSTTY_LOG"), None);
        assert!(session.spec.tracked_env.is_empty());
    }
}

#[test]
fn an_attach_the_cli_would_refuse_is_refused_with_its_status() {
    // inside another session (the launch keeps the inherited name)
    let inside = PaneCommand::new("/bin/sh").env("NGTERM_LIVE_DIR", "/tmp/ns");
    let refusal = resolve(&attach(&[]), &inside, base(), None).unwrap_err();
    assert_eq!(refusal.code, EXIT_FAILED);
    assert!(
        refusal
            .message
            .as_deref()
            .is_some_and(|message| message.contains("inside the live session outer")),
        "{refusal:?}"
    );
    // no namespace
    let nowhere = PaneCommand::new("/bin/sh").env("NGTERM_LIVE_SESSION", "");
    let refusal = resolve(&attach(&[]), &nowhere, base(), None).unwrap_err();
    assert_eq!(refusal.code, EXIT_USAGE);
    // no session name
    let unnamed = LiveAttach {
        session: String::new(),
        ..attach(&[])
    };
    assert_eq!(
        resolve(&unnamed, &command(), base(), None).unwrap_err().code,
        EXIT_USAGE
    );
    // the app's directory serves a launch without one
    let here = PaneCommand { cwd: None, ..command() };
    let session = resolve(&attach(&[]), &here, base(), Some(PathBuf::from("/app"))).unwrap();
    assert_eq!(session.spec.cwd.as_deref(), Some("/app"));
    // a relative one is the app's, as a plain pane's child resolves it (the server runs from the filesystem root)
    let relative = command().cwd("work/tree");
    let session = resolve(&attach(&[]), &relative, base(), Some(PathBuf::from("/app"))).unwrap();
    assert_eq!(
        session.spec.cwd.map(PathBuf::from),
        Some(Path::new("/app").join("work").join("tree"))
    );
}

#[test]
fn the_childs_environment_is_the_apps_under_the_launchs() {
    let command = PaneCommand::new("x").env("B", "launch").env("C", "new").env_remove("A");
    let env = child_environment(
        &command,
        [("A", "1"), ("B", "2"), ("D", "4")].map(|(key, value)| (key.to_owned(), value.to_owned())),
    );
    assert_eq!(lookup(&env, "A"), None);
    assert_eq!(lookup(&env, "B"), Some("launch"));
    assert_eq!(lookup(&env, "C"), Some("new"));
    assert_eq!(lookup(&env, "D"), Some("4"));
    assert_eq!(env.iter().filter(|(key, _)| key == "B").count(), 1);
    // where names ignore case a launch's `PATH` replaces the app's `Path`
    let path = child_environment(
        &PaneCommand::new("x").env("PATH", "launch"),
        [("Path".to_owned(), "app".to_owned())],
    );
    if cfg!(windows) {
        assert_eq!(path, [("PATH".to_owned(), "launch".to_owned())]);
    } else {
        assert_eq!(path.len(), 2);
    }
}

#[test]
fn only_whole_kitty_answers_are_kept_for_a_barrier() {
    let mut answers = Vec::new();
    keep_kitty_answer(&mut answers, b"\x1b_Gi=1;OK\x1b\\");
    keep_kitty_answer(&mut answers, b"\x1b[?62;22c");
    keep_kitty_answer(&mut answers, b"\x1b]11;rgb:0000/0000/0000\x1b\\");
    keep_kitty_answer(&mut answers, b"\x1b_Gi=2;ENOENT:gone\x1b\\");
    assert_eq!(answers, b"\x1b_Gi=1;OK\x1b\\\x1b_Gi=2;ENOENT:gone\x1b\\");
    let mut full = vec![b'x'; KITTY_ANSWERS_LIMIT];
    keep_kitty_answer(&mut full, b"\x1b_Gi=3;OK\x1b\\");
    assert_eq!(full.len(), KITTY_ANSWERS_LIMIT, "past the limit an answer is dropped");
}

#[test]
fn input_waits_within_the_write_limit_and_none_is_taken_after_leaving() {
    let live = Live::new(ReadPause::new(), 8, true);
    live.input(b"0123456789").unwrap();
    assert_eq!(live.queued(), 10, "an empty queue takes a write whole");
    assert_eq!(live.input(b"x"), Err(PaneError::Backpressure));
    // the pane's own input goes past the limit, uncounted
    live.program_input(b"\x0c");
    assert_eq!(live.queued(), 10);
    live.leave();
    assert_eq!(live.input(b"x"), Err(PaneError::Closed));
    assert_eq!(live.queued(), 0);
}

#[test]
fn facts_are_sent_only_when_they_change_and_an_unknown_answer_is_dropped() {
    let live = Live::new(ReadPause::new(), 1024, true);
    let facts = ReplyMetadata {
        cell_width_px: 8,
        ..ReplyMetadata::default()
    };
    live.update_metadata(facts.clone());
    live.update_metadata(facts.clone());
    assert_eq!(lock(&live.outbox).queue.len(), 1);
    assert_eq!(*lock(&live.metadata), facts);
    live.answer(7, ClipboardAnswer::Denied);
    assert_eq!(lock(&live.outbox).queue.len(), 1, "no read 7 waits");
    live.resize(GridSize {
        cols: 80,
        rows: 24,
        cell_width: 8,
        cell_height: 16,
    });
    let queued: Vec<String> = lock(&live.outbox)
        .queue
        .iter()
        .map(|message| format!("{message:?}"))
        .collect();
    assert!(queued[1].starts_with("Resize"), "{queued:?}");
}

#[test]
fn a_launch_that_clears_the_environment_keeps_only_its_own_variables() {
    let command = PaneCommand::new("x").env_clear().env("B", "launch");
    let env = child_environment(&command, [("A".to_owned(), "1".to_owned())]);
    assert_eq!(env, [("B".to_owned(), "launch".to_owned())]);
}

#[test]
fn a_live_outlet_keeps_kitty_answers_and_sends_the_panes_own_input() {
    let live = Arc::new(Live::new(ReadPause::new(), 1024, true));
    let outlet = crate::outlet::ReplyOutlet::default();
    outlet.go_live(Arc::clone(&live));
    outlet.push_reply(b"\x1b[0n");
    outlet.push_reply(b"\x1b_Gi=1;OK\x1b\\");
    outlet.push_input(b"\x0c");
    assert_eq!(outlet.take_kitty_answers(), b"\x1b_Gi=1;OK\x1b\\");
    assert!(outlet.take_kitty_answers().is_empty(), "taken");
    let queued: Vec<String> = lock(&live.outbox)
        .queue
        .iter()
        .map(|message| format!("{message:?}"))
        .collect();
    assert_eq!(queued, [format!("{:?}", Message::Input(b"\x0c".to_vec()))]);
    // the program's own input is not the pane's to count against its write limit
    assert_eq!(live.queued(), 0);
}

#[test]
fn nothing_is_queued_once_the_pane_left() {
    let live = Live::new(ReadPause::new(), 1024, true);
    live.leave();
    live.resize(GridSize {
        cols: 80,
        rows: 24,
        cell_width: 8,
        cell_height: 16,
    });
    live.program_input(b"\x0c");
    live.update_metadata(ReplyMetadata {
        cell_width_px: 9,
        ..ReplyMetadata::default()
    });
    assert!(lock(&live.outbox).queue.is_empty());
    assert!(live.next_message().is_none(), "the sender ends");
}

#[test]
fn a_paused_reader_waits_for_the_resume_or_the_pane_leaving() {
    let pause = ReadPause::new();
    let live = Arc::new(Live::new(pause.clone(), 1024, true));
    assert!(live.wait_resumed(), "not paused: no wait");
    for leaving in [false, true] {
        pause.pause();
        let (started, ready) = std::sync::mpsc::channel();
        let (finished, result) = std::sync::mpsc::channel();
        let waiting = {
            let live = Arc::clone(&live);
            std::thread::spawn(move || {
                started.send(()).unwrap();
                finished.send(live.wait_resumed()).unwrap();
            })
        };
        ready.recv_timeout(std::time::Duration::from_secs(20)).unwrap();
        let before_resume = result.recv_timeout(std::time::Duration::from_millis(100));
        if leaving {
            live.leave();
        } else {
            pause.resume();
            live.wake_reader();
        }
        let after_resume = result.recv_timeout(std::time::Duration::from_secs(20));
        waiting.join().unwrap();
        assert_eq!(before_resume, Err(std::sync::mpsc::RecvTimeoutError::Timeout));
        assert_eq!(after_resume.unwrap(), !leaving);
    }
}

const SIZE: GridSize = GridSize {
    cols: 40,
    rows: 4,
    cell_width: 8,
    cell_height: 16,
};

/// A stream pane: a real terminal and IO worker, with no PTY, session or thread of a channel's.
fn pane() -> crate::Pane {
    crate::Pane::stream(&crate::PaneConfig::new(SIZE)).unwrap().0
}

fn delegated(id: u64, selection: &str, list_only: bool) -> LiveMail {
    LiveMail::Delegated {
        request: Delegated {
            native_alias: None,
            id,
            epoch: 1,
            request: DelegatedRequest::ClipboardRead {
                selection: selection.to_owned(),
                list_only,
            },
        },
        connection: 1,
    }
}

fn clipboard_reads(pane: &crate::Pane) -> Vec<ClipboardReadRequest> {
    pane.take_events()
        .into_iter()
        .filter_map(|event| match event {
            PaneEvent::ClipboardRead(read) => Some(read),
            _ => None,
        })
        .collect()
}

#[test]
fn a_delegated_read_is_asked_of_the_frontend_and_its_answer_sent_on_its_connection() {
    let pane = pane();
    let live = Live::new(ReadPause::new(), 1024, true);
    handle(&live, pane.shared(), delegated(10, "s", false));
    handle(&live, pane.shared(), delegated(11, "p", true));
    handle(&live, pane.shared(), delegated(12, "c", false));
    let reads = clipboard_reads(&pane);
    assert_eq!(
        reads.iter().map(|read| (read.location, read.scope)).collect::<Vec<_>>(),
        [
            (ClipboardLocation::Selection, ReadScope::Data),
            (ClipboardLocation::Primary, ReadScope::ListOnly),
            (ClipboardLocation::Standard, ReadScope::Data),
        ]
    );
    live.answer(reads[0].id, ClipboardAnswer::Text(Some("copied".to_owned())));
    live.answer(reads[0].id, ClipboardAnswer::Denied);
    let queued: Vec<String> = lock(&live.outbox)
        .queue
        .iter()
        .map(|message| format!("{message:?}"))
        .collect();
    assert_eq!(queued.len(), 1, "answered once: {queued:?}");
    assert!(queued[0].contains("Clipboard { text: Some(\"copied\") }"), "{queued:?}");
    assert!(
        queued[0].contains("id: 10") && queued[0].contains("connection: 1"),
        "{queued:?}"
    );
    live.answer(reads[2].id, ClipboardAnswer::Denied);
    let denied = format!("{:?}", lock(&live.outbox).queue[1]);
    assert!(
        denied.contains("id: 12") && denied.contains("reply: Denied"),
        "{denied}"
    );
    // a lost connection's requests are nobody's
    live.lost();
    live.answer(reads[1].id, ClipboardAnswer::Denied);
    assert_eq!(lock(&live.outbox).queue.len(), 2);
}

#[test]
fn the_facts_are_the_terminals_with_the_kitty_graphics_shown_and_the_scheme() {
    let pane = pane();
    let live = Live::new(ReadPause::new(), 1024, true);
    let facts = live.metadata_of(pane.shared(), ColorScheme::Light);
    assert_eq!((facts.cell_width_px, facts.cell_height_px), (8, 16));
    assert!(facts.kitty_graphics);
    assert_eq!(facts.color_scheme, Some(wire::ColorScheme::Light));
    assert_eq!(facts.palette.len(), 256);
    live.set_kitty_graphics(false);
    let facts = live.metadata_of(pane.shared(), ColorScheme::Dark);
    assert!(!facts.kitty_graphics);
    assert_eq!(facts.color_scheme, Some(wire::ColorScheme::Dark));
}

fn wait_exit(pane: &crate::Pane, events: &mut Vec<PaneEvent>) -> crate::ExitStatus {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        events.extend(pane.take_events());
        if let Some(status) = events.iter().find_map(|event| match event {
            PaneEvent::Exited(status) => Some(status.clone()),
            _ => None,
        }) {
            return status;
        }
        assert!(std::time::Instant::now() < deadline, "no exit: {events:?}");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn a_refused_attach_writes_why_and_ends_the_pane_with_the_clis_status() {
    let pane = crate::Pane::attach_live(
        Err(Outcome::failed("ngterm-session: no session here")),
        &crate::PaneConfig::new(SIZE),
    )
    .unwrap();
    let status = wait_exit(&pane, &mut Vec::new());
    assert_eq!(status.code, u32::from(EXIT_FAILED));
    assert_eq!(status.signal, None);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let lines = loop {
        let snapshot = pane.frame();
        if !snapshot.stale {
            break snapshot.frame.lines();
        }
        assert!(std::time::Instant::now() < deadline, "complaint frame stayed stale");
        std::thread::yield_now();
    };
    assert!(
        lines
            .iter()
            .any(|line| line.contains("ngterm-session: no session here")),
        "{lines:?}"
    );
}

#[test]
fn a_pane_that_left_is_not_ended_by_its_attach_failing() {
    let pane = pane();
    let live = Arc::new(Live::new(ReadPause::new(), 1024, true));
    live.leave();
    Reader {
        live,
        shared: Arc::clone(pane.shared()),
        session: Err(Outcome::failed("refused")),
    }
    .run();
    // the complaint is written, but nothing ends the pane that is going away
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !pane.frame().frame.lines().iter().any(|line| line == "refused") {
        assert!(std::time::Instant::now() < deadline, "no complaint");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        !pane
            .take_events()
            .iter()
            .any(|event| matches!(event, PaneEvent::Exited(_))),
        "no exit reported"
    );
}
