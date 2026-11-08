use super::*;

#[test]
fn consumer_ids_keep_all_bits_including_zero() {
    for raw in [0, 1, u64::MAX] {
        assert_eq!(SessionId::from_raw(raw).raw(), raw);
        assert_eq!(ViewId::from_raw(raw).raw(), raw);
        assert_eq!(BufferId::from_raw(raw).raw(), raw);
        assert_eq!(PolicyToken::from_raw(raw).raw(), raw);
        assert_eq!(ReplyToken::from_raw(raw).raw(), raw);
        assert_eq!(SchemeToken::from_raw(raw).raw(), raw);
        assert_eq!(MenuToken::from_raw(raw).raw(), raw);
    }
}

/// Runs a native `WebKit` case alone: its globals belong to one thread for the process's lifetime.
///
/// The child passes with its own exit code: a `name` that selects no test exits 0 without running anything.
pub(crate) fn isolated(name: &str, run: impl FnOnce()) {
    const CHILD: &str = "WPE_UNIT_CHILD";
    const PASSED: i32 = 77;
    if std::env::var(CHILD).as_deref() == Ok(name) {
        let passed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).is_ok();
        crate::exit_process(if passed { PASSED } else { 101 });
    }
    let home = tempfile::tempdir().unwrap();
    let mut command = crate::launcher::command(&std::env::current_exe().unwrap());
    command.args(["--exact", name, "--nocapture"]);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") || key.to_string_lossy().starts_with("AGTERM_") {
            command.env_remove(key);
        }
    }
    command.env(CHILD, name);
    for (key, dir) in [
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
    ] {
        command.env(key, home.path().join(dir));
    }
    let mut child = command.spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(PASSED), "{name} did not run and pass: {status}");
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("{name} exceeded its deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
