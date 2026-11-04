//! `scripts/ios-sim-runner.sh`, cargo's runner of the `aarch64-apple-ios-sim` target: it boots the pinned phone
//! simulator, runs the executable there through `xcrun simctl spawn` with its arguments, forwards `RUST_BACKTRACE` and
//! `RUST_TEST_*` (as `SIMCTL_CHILD_*`) and nothing else, and exits with the executable's status.
//!
//! Every case runs the real script with stub `uname` and `xcrun` first on a `PATH` that holds nothing else but
//! `/usr/bin` and `/bin`, in a cleared environment, so nothing here needs a simulator; the stub `xcrun` logs each call,
//! and its `simctl spawn` prints what the spawned process would get and exits with the status the case asks for. The
//! real runner on a real simulator is what `cargo test --target aarch64-apple-ios-sim` uses (Task 2 of the iOS plan).
//! Unix hosts only: the script is macOS-only, and the stub `uname` lets Linux run it too.
#![cfg(unix)]

/// The stubs, the scratch directory and the run of the script.
#[cfg(test)]
mod host {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    pub(super) const UDID: &str = "0F6F99AC-7E30-476E-BCD5-248A5CD003BC";

    pub(super) fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// The value of the one `NAME="..."` line of build/pins.env.
    pub(super) fn pin(name: &str) -> String {
        let pins = fs::read_to_string(repo_root().join("build/pins.env")).unwrap();
        let prefix = format!("{name}=\"");
        let values: Vec<&str> = pins
            .lines()
            .filter_map(|line| line.strip_prefix(&prefix)?.strip_suffix('"'))
            .collect();
        assert_eq!(values.len(), 1, "build/pins.env pins {name} once");
        values[0].to_owned()
    }

    fn write_stub(dir: &Path, name: &str, body: &str) {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    pub(super) struct Run {
        pub(super) output: Output,
        /// Each `xcrun` call, one per line.
        pub(super) log: String,
        /// The executable the case handed the runner, as an absolute path.
        pub(super) executable: PathBuf,
    }

    impl Run {
        pub(super) fn stdout(&self) -> String {
            String::from_utf8_lossy(&self.output.stdout).into_owned()
        }

        pub(super) fn stderr(&self) -> String {
            String::from_utf8_lossy(&self.output.stderr).into_owned()
        }
    }

    /// The runner over `devices` (the `simctl list devices` text), run from a scratch directory on `executable`
    /// (relative to it) and `args`, with `env` set, its `simctl spawn` exiting with `status`.
    pub(super) fn run(devices: &str, executable: &str, args: &[&str], env: &[(&str, &str)], status: i32) -> Run {
        let scratch = tempfile::tempdir().unwrap();
        let stubs = scratch.path().join("stubs");
        fs::create_dir_all(&stubs).unwrap();
        fs::create_dir_all(scratch.path().join("deps")).unwrap();
        fs::write(scratch.path().join("deps/tests"), "").unwrap();
        write_stub(&stubs, "uname", "echo Darwin\n");
        let log = scratch.path().join("xcrun.log");
        fs::write(&log, "").unwrap();
        write_stub(
            &stubs,
            "xcrun",
            &format!(
                r#"echo "$*" >>"$STUB_LOG"
if [ "$1 $2 $3" = 'simctl list devices' ]; then printf '%s\n' '{devices}'; exit 0; fi
if [ "$1 $2" = 'simctl spawn' ]; then
  env | grep '^SIMCTL_CHILD_' | sort
  exit {status}
fi
exit 0
"#
            ),
        );
        let mut command = Command::new("bash");
        command
            .arg(repo_root().join("scripts/ios-sim-runner.sh"))
            .arg(executable)
            .args(args)
            .current_dir(scratch.path())
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", stubs.display()))
            .env("HOME", scratch.path())
            .env("STUB_LOG", &log);
        for (name, value) in env {
            command.env(name, value);
        }
        let output = command.output().unwrap();
        // the runner's $PWD is the physical directory (the environment has no PWD), and macOS's temporary directory
        // is behind a symlink
        let executable = fs::canonicalize(scratch.path()).unwrap().join(executable);
        Run {
            output,
            log: fs::read_to_string(&log).unwrap(),
            executable,
        }
    }

    /// `simctl list devices` with the pinned phone on the pinned runtime, beside a same-named one on another runtime.
    pub(super) fn devices() -> String {
        let name = pin("IOS_SIM_PHONE_NAME");
        format!(
            "== Devices ==\n-- iOS 18.0 --\n    {name} (11111111-2222-3333-4444-555555555555) (Shutdown)\n\
             -- iOS {} --\n    other (99999999-2222-3333-4444-555555555555) (Shutdown)\n    {name} ({UDID}) \
             (Shutdown)\n",
            pin("IOS_SIMULATOR_RUNTIME")
        )
    }
}

use std::process::Command;

use host::{UDID, devices, pin, repo_root, run};

#[test]
fn it_runs_the_executable_on_the_pinned_phone_and_forwards_only_rusts_test_variables() {
    let run = run(
        &devices(),
        "deps/tests",
        &["--exact", "a test"],
        &[
            ("RUST_BACKTRACE", "1"),
            ("RUST_TEST_THREADS", "3"),
            ("RUST_TEST_NOCAPTURE", "1"),
            ("RUST_LOG", "debug"),
            ("OTHER", "x"),
        ],
        0,
    );
    assert!(run.output.status.success(), "{}", run.stderr());
    let lines: Vec<String> = run.log.lines().map(str::to_owned).collect();
    assert_eq!(
        lines,
        [
            "simctl list devices".to_owned(),
            format!("simctl bootstatus {UDID} -b"),
            format!("simctl spawn {UDID} {} --exact a test", run.executable.display()),
        ],
        "boots the pinned runtime's simulator, then spawns the executable on it by absolute path"
    );
    let stdout = run.stdout();
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("SIMCTL_CHILD_"))
            .collect::<Vec<_>>(),
        [
            "SIMCTL_CHILD_RUST_BACKTRACE=1",
            "SIMCTL_CHILD_RUST_TEST_NOCAPTURE=1",
            "SIMCTL_CHILD_RUST_TEST_THREADS=3",
        ],
        "{stdout}"
    );
}

#[test]
fn it_exits_with_the_executables_status() {
    for status in [1, 101, 134] {
        let run = run(&devices(), "deps/tests", &[], &[], status);
        assert_eq!(run.output.status.code(), Some(status), "{}", run.stderr());
    }
}

#[test]
fn a_missing_simulator_names_the_setup_script_and_spawns_nothing() {
    let elsewhere = format!(
        "== Devices ==\n-- iOS 18.0 --\n    {} ({UDID}) (Shutdown)\n",
        pin("IOS_SIM_PHONE_NAME")
    );
    let run = run(&elsewhere, "deps/tests", &[], &[], 0);
    assert!(!run.output.status.success());
    assert!(run.stderr().contains("scripts/setup-ios.sh"), "{}", run.stderr());
    assert!(!run.log.contains("spawn"), "{}", run.log);
    assert!(!run.log.contains("bootstatus"), "{}", run.log);
}

#[test]
fn a_missing_executable_or_no_arguments_fails_before_the_simulator() {
    let missing = run(&devices(), "deps/absent", &[], &[], 0);
    assert!(!missing.output.status.success());
    assert!(
        missing.stderr().contains("no executable at deps/absent"),
        "{}",
        missing.stderr()
    );
    assert_eq!(missing.log, "");
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/ios-sim-runner.sh"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage:"));
}
