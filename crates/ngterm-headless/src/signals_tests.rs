use super::ignored_at_start;

#[test]
fn a_signal_ignored_before_the_handlers_is_read_back() {
    // SIGUSR2: nothing in the tests sends it, so its action is this test's to set and put back
    let signal = libc::SIGUSR2;
    // SAFETY: `SIG_IGN` installs no handler code.
    let previous = unsafe { libc::signal(signal, libc::SIG_IGN) };
    assert!(ignored_at_start(signal));
    // SAFETY: `SIG_DFL` installs no handler code.
    unsafe { libc::signal(signal, libc::SIG_DFL) };
    assert!(!ignored_at_start(signal));
    // SAFETY: the action read back above, put back as it was.
    unsafe { libc::signal(signal, previous) };
}

/// Signal handlers are process-wide; each scenario runs in a fresh test process.
#[test]
fn termination_posts_an_orderly_quit_and_ignored_hangup_stays_ignored() {
    use std::io::{BufRead as _, BufReader};
    use std::process::{Command, Stdio};
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "signals::tests::signal_child", "--nocapture"])
            .env("HEADLESS_SIGNAL_CHILD", signal.to_string())
            .stdout(Stdio::piped());
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("NGTERM_") {
                command.env_remove(name);
            }
        }
        let mut child = command.spawn().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            assert_ne!(
                output.read_line(&mut line).unwrap(),
                0,
                "child ended before ready: {line}"
            );
            if line.contains("SIGNAL_READY") {
                break;
            }
            line.clear();
        }
        // SAFETY: this is the test's own child, and the signal number is one of the three above.
        assert_eq!(unsafe { libc::kill(i32::try_from(child.id()).unwrap(), signal) }, 0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "{status}");
                break;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("signal child did not finish");
            }
            std::thread::yield_now();
        }
    }
}

#[test]
fn signal_child() {
    use ngterm_host::MainLoop;
    use std::io::Write as _;
    use std::sync::Arc;
    let Ok(value) = std::env::var("HEADLESS_SIGNAL_CHILD") else {
        return;
    };
    let signal: i32 = value.parse().unwrap();
    for kind in [libc::SIGINT, libc::SIGHUP] {
        let disposition = if kind == signal { libc::SIG_DFL } else { libc::SIG_IGN };
        // SAFETY: this isolated process owns the signal dispositions.
        unsafe {
            libc::signal(kind, disposition);
        }
    }
    let main_loop = Arc::new(crate::main_loop::ThreadMainLoop::<crate::state::HeadlessState>::default());
    let posting: Arc<dyn MainLoop<crate::state::HeadlessState>> = main_loop.clone();
    let poster = Arc::downgrade(&posting);
    super::quit_on_termination(poster.clone()).unwrap();
    if signal != libc::SIGHUP {
        assert!(ignored_at_start(libc::SIGHUP));
    }
    if signal != libc::SIGINT {
        assert!(ignored_at_start(libc::SIGINT));
    }
    let config = ngterm_core::ghostty_config::GhosttyConfig::default();
    let core = ngterm_core::Core::new(
        ngterm_core::CoreConfig {
            app: crate::identity(),
            capabilities: Vec::new(),
        },
        ngterm_core::CoreState::default(),
    );
    let panes = crate::panes::Panes::new(
        crate::panes::PaneEnvironment {
            socket: "unused".into(),
            program_version: "test".into(),
            host: ngterm_term::Host::new(&config, None, None, None),
            config,
            grid: (80, 24),
            hostname: "box".into(),
        },
        Arc::new(|_, _| {}),
    );
    let mut state = crate::state::HeadlessState::new(core, poster, panes);
    println!("SIGNAL_READY");
    std::io::stdout().flush().unwrap();
    main_loop.run(&mut state, |state| {
        if state.quit_requested() {
            main_loop.close();
        }
    });
    assert!(state.quit_requested());
}
