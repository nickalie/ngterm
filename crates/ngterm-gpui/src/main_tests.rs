use super::*;

#[test]
fn identity_query_requires_the_single_exact_argument() {
    for args in [
        vec![],
        vec!["--other"],
        vec![app::BUILD_IDENTITY_FLAG, "extra"],
        vec!["other", app::BUILD_IDENTITY_FLAG],
    ] {
        assert!(!is_build_identity_query(args.into_iter().map(OsString::from)));
    }
    assert!(is_build_identity_query(
        [OsString::from(app::BUILD_IDENTITY_FLAG)].into_iter()
    ));
}

#[test]
fn state_directory_claim_refuses_a_second_owner_and_an_unusable_path() {
    claims_refuse_a_second_owner_and_an_unusable_path();
}

/// The claim's checks hold while another thread of this harness spawns children, as this binary's other tests do: a
/// child holds a duplicate of every descriptor of the harness from its fork until its exec.
#[test]
fn the_state_directory_claims_hold_while_other_tests_spawn_children() {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let spawner = std::thread::spawn({
        let stop = std::sync::Arc::clone(&stop);
        move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                spawn_a_child();
            }
        }
    });
    let checked = std::panic::catch_unwind(|| {
        for _ in 0..200 {
            claims_refuse_a_second_owner_and_an_unusable_path();
        }
    });
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    spawner.join().unwrap();
    if let Err(panic) = checked {
        std::panic::resume_unwind(panic);
    }
}

/// Runs a child of this binary that lists no test, as this binary's other tests spawn theirs.
fn spawn_a_child() {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") {
            command.env_remove(key);
        }
    }
    let _ = command
        .args(["--list", "--exact", "tests::no_such_test"])
        .stdout(std::process::Stdio::null())
        .status();
}

/// The lock's release is not checked here: a child another test forks keeps it until its exec, so a claim right after
/// the drop can be refused (`ngterm_core::state_lock`'s tests check it alone in a fresh process).
fn claims_refuse_a_second_owner_and_an_unusable_path() {
    let dir = tempfile::tempdir().unwrap();
    let _lock = claim_state_dir(dir.path()).unwrap();
    assert!(matches!(claim_state_dir(dir.path()), Err(ExitCode::FAILURE)));
    let file = dir.path().join("file");
    std::fs::write(&file, b"not a directory").unwrap();
    assert!(matches!(claim_state_dir(&file.join("state")), Err(ExitCode::FAILURE)));
}

#[test]
fn immediate_exit_preserves_the_status_and_flushes_output() {
    const CHILD: &str = "GPUI_COVERAGE_EXIT_CHILD";
    if std::env::var_os(CHILD).is_some() {
        print!("flushed before exit");
        exit_now(23);
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") {
            command.env_remove(key);
        }
    }
    let output = command
        .args([
            "--exact",
            "tests::immediate_exit_preserves_the_status_and_flushes_output",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(23));
    assert!(String::from_utf8_lossy(&output.stdout).contains("flushed before exit"));
}

#[cfg(target_os = "linux")]
#[test]
fn forwarding_delivers_activation_to_a_private_instance() {
    let dir = tempfile::tempdir().unwrap();
    let app_id = format!("com.nickalie.ngterm.test_{}", uuid::Uuid::new_v4().simple());
    let (requests, received) = async_channel::unbounded();
    let Registration::Primary(primary) = instance::register(&app_id, dir.path(), requests).unwrap() else {
        panic!("the private bus accepts the primary")
    };
    let (requests, _) = async_channel::unbounded();
    let Registration::Remote(remote) = instance::register(&app_id, dir.path(), requests).unwrap() else {
        panic!("the next registration finds the primary")
    };
    let activation = Activation {
        token: Some("test-token".into()),
        startup_id: Some("test_TIME1".into()),
    };
    assert_eq!(forward(&remote, &activation), ExitCode::SUCCESS);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let request = loop {
        if let Ok(request) = received.try_recv() {
            break request;
        }
        assert!(std::time::Instant::now() < deadline, "activation was not delivered");
        std::thread::yield_now();
    };
    assert_eq!(request, instance::BusRequest::Activate(activation));
    primary.close().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn a_broken_bus_connection_makes_forwarding_fail() {
    use std::io::BufRead as _;
    use std::process::{Command, Stdio};

    const BUS_PID: &str = "GPUI_COVERAGE_PRIVATE_BUS_PID";
    if let Ok(pid) = std::env::var(BUS_PID) {
        forward_after_bus_stops(pid.parse().unwrap());
        return;
    }
    let mut bus_command = Command::new("dbus-daemon");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") {
            bus_command.env_remove(key);
        }
    }
    let mut bus = bus_command
        .args(["--session", "--nofork", "--print-address=1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut address = String::new();
    std::io::BufReader::new(bus.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") {
            child.env_remove(key);
        }
    }
    let output = child
        .args([
            "--exact",
            "tests::a_broken_bus_connection_makes_forwarding_fail",
            "--nocapture",
        ])
        .env("DBUS_SESSION_BUS_ADDRESS", address.trim())
        .env(BUS_PID, bus.id().to_string())
        .output();
    let _ = bus.kill();
    bus.wait().unwrap();
    let output = output.unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
fn forward_after_bus_stops(pid: libc::pid_t) {
    let dir = tempfile::tempdir().unwrap();
    let app_id = format!("com.nickalie.ngterm.test_{}", uuid::Uuid::new_v4().simple());
    let (requests, _) = async_channel::unbounded();
    let Registration::Primary(_primary) = instance::register(&app_id, dir.path(), requests).unwrap() else {
        panic!("private primary")
    };
    let (requests, _) = async_channel::unbounded();
    let Registration::Remote(remote) = instance::register(&app_id, dir.path(), requests).unwrap() else {
        panic!("private remote")
    };
    // SAFETY: the parent test started this private bus and keeps its child handle until this process ends.
    let stopped = unsafe { libc::kill(pid, libc::SIGTERM) };
    assert_eq!(stopped, 0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if forward(&remote, &Activation::default()) == ExitCode::FAILURE {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "forwarding kept succeeding after the bus stopped"
        );
        std::thread::yield_now();
    }
}
