//! Probing reads only the loaded library version, without starting an engine or touching app state.

#[test]
fn the_probe_prints_the_loaded_library_version_without_an_engine() {
    let home = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_ngterm-web"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("NGTERM_") {
            command.env_remove(name);
        }
    }
    let output = command
        .arg("--probe")
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("LD_LIBRARY_PATH", "/ngterm-test-no-libraries")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    // SAFETY: these version getters have no preconditions and start no engine.
    let major = unsafe { ngterm_wpe_sys::webkit_get_major_version() };
    // SAFETY: as above.
    let minor = unsafe { ngterm_wpe_sys::webkit_get_minor_version() };
    // SAFETY: as above.
    let micro = unsafe { ngterm_wpe_sys::webkit_get_micro_version() };
    let expected = format!("{major}.{minor}.{micro}\n");
    assert_eq!(output.stdout, expected.as_bytes());
    ngterm_wpe_client::worker::pin_worker_path(std::path::Path::new(env!("CARGO_BIN_EXE_ngterm-web"))).unwrap();
    assert_eq!(
        ngterm_wpe_client::worker::probe_system().as_deref(),
        Some(expected.trim())
    );
    assert!(output.stderr.is_empty());
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
}
