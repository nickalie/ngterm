//! `scripts/setup-ios.sh --check`, the gate of the iOS build: it passes, printing the exports, only when the pinned
//! Xcode, both Rust iOS targets, the pinned simulator runtime and the pinned `SwiftLint` are in place, and otherwise
//! names everything that is missing and exits non-zero. It never changes anything.
//!
//! Every case runs the real script with stub `uname`, `xcode-select`, `xcodebuild`, `xcrun`, `rustup` and `swiftlint`
//! first on a `PATH` that holds nothing else but `/usr/bin` and `/bin`, in a cleared environment with a scratch `HOME`
//! and vendor root, so nothing here depends on the host's toolchain, and the stubs log every call that would change
//! something. Unix hosts only: the script is macOS-only, and the stub `uname` lets Linux run it too.
#![cfg(unix)]

/// The stubs, the scratch host and the run of the script.
#[cfg(test)]
mod host {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// The value of the one `NAME="..."` line of build/pins.env.
    pub(super) fn pin(name: &str) -> String {
        let pins = fs::read_to_string(repo_root().join("build/pins.env")).unwrap();
        let prefix = format!("{name}=\"");
        let values: Vec<&str> = pins
            .lines()
            .filter_map(|line| line.strip_prefix(&prefix))
            .filter_map(|rest| rest.strip_suffix('"'))
            .collect();
        assert_eq!(values.len(), 1, "build/pins.env pins {name} once");
        values[0].to_owned()
    }

    /// The `xcrun simctl list runtimes` line of an installed iOS runtime of VERSION.
    pub(super) fn runtime_line(version: &str) -> String {
        format!(
            "iOS {version} ({version} - 24A100) - com.apple.CoreSimulator.SimRuntime.iOS-{}",
            version.replace('.', "-")
        )
    }

    /// What the stubs report: the host as the pins want it unless a case changes one field.
    pub(super) struct Host {
        pub(super) os: &'static str,
        pub(super) xcode: String,
        pub(super) runtimes: Vec<String>,
        pub(super) rust_targets: Vec<&'static str>,
        pub(super) swiftlint: Option<String>,
    }

    impl Host {
        pub(super) fn pinned() -> Self {
            Self {
                os: "Darwin",
                xcode: pin("IOS_XCODE_VERSION"),
                runtimes: vec![runtime_line(&pin("IOS_SIMULATOR_RUNTIME"))],
                rust_targets: vec!["aarch64-apple-darwin", "aarch64-apple-ios", "aarch64-apple-ios-sim"],
                swiftlint: Some(pin("SWIFTLINT_VERSION")),
            }
        }
    }

    fn write_stub(dir: &Path, name: &str, body: &str) {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A call that changes something, appended to the log.
    const LOG_CALL: &str = r#"echo "$(basename "$0") $*" >>"$STUB_LOG""#;

    pub(super) struct Run {
        pub(super) output: Output,
        pub(super) log: String,
        pub(super) developer_dir: PathBuf,
    }

    impl Run {
        pub(super) fn stderr(&self) -> String {
            String::from_utf8_lossy(&self.output.stderr).into_owned()
        }

        pub(super) fn stdout(&self) -> String {
            String::from_utf8_lossy(&self.output.stdout).into_owned()
        }

        pub(super) fn assert_fails_naming(&self, needles: &[&str]) {
            let stderr = self.stderr();
            assert!(!self.output.status.success(), "--check passed:\n{stderr}");
            for needle in needles {
                assert!(stderr.contains(needle), "stderr lacks {needle:?}:\n{stderr}");
            }
            assert!(
                stderr.contains("scripts/setup-ios.sh"),
                "stderr names no fix:\n{stderr}"
            );
            assert!(
                self.stdout().is_empty(),
                "a failed check printed exports:\n{}",
                self.stdout()
            );
        }
    }

    pub(super) fn check(host: &Host) -> Run {
        let scratch = tempfile::tempdir().unwrap();
        let stubs = scratch.path().join("stubs");
        let developer_dir = scratch.path().join("Xcode.app/Contents/Developer");
        fs::create_dir_all(&stubs).unwrap();
        fs::create_dir_all(&developer_dir).unwrap();
        write_stub(
            &stubs,
            "uname",
            &format!("case \"$1\" in -m) echo arm64 ;; *) echo {} ;; esac\n", host.os),
        );
        write_stub(
            &stubs,
            "xcode-select",
            "[ \"$1\" = -p ] && echo \"$STUB_DEVELOPER_DIR\"\n",
        );
        write_stub(
            &stubs,
            "xcodebuild",
            &format!(
                "if [ \"$1\" = -version ]; then printf 'Xcode %s\\nBuild version 27B52\\n' {}; exit 0; fi\n{LOG_CALL}\n",
                host.xcode
            ),
        );
        write_stub(
            &stubs,
            "xcrun",
            &format!(
                "if [ \"$1 $2 $3\" = 'simctl list runtimes' ]; then printf '== Runtimes ==\\n'; printf '%s\\n' {}; exit 0; fi\n\
                 {LOG_CALL}\n",
                host.runtimes
                    .iter()
                    .map(|line| format!("'{line}'"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        );
        write_stub(
            &stubs,
            "rustup",
            &format!(
                "if [ \"$1 $2 $3\" = 'target list --installed' ]; then printf '%s\\n' {}; exit 0; fi\n{LOG_CALL}\n",
                host.rust_targets.join(" ")
            ),
        );
        if let Some(version) = &host.swiftlint {
            write_stub(
                &stubs,
                "swiftlint",
                &format!("[ \"$1\" = version ] && echo {version}\n"),
            );
        }
        let log = scratch.path().join("calls.log");
        fs::write(&log, "").unwrap();
        let output = Command::new("bash")
            .arg(repo_root().join("scripts/setup-ios.sh"))
            .arg("--check")
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", stubs.display()))
            .env("HOME", scratch.path())
            .env("TMPDIR", scratch.path())
            .env("NGTERM_VENDOR", scratch.path().join("vendor"))
            .env("STUB_DEVELOPER_DIR", &developer_dir)
            .env("STUB_LOG", &log)
            .output()
            .unwrap();
        Run {
            output,
            log: fs::read_to_string(&log).unwrap(),
            developer_dir,
        }
    }
}

use host::{Host, check, pin, runtime_line};

#[test]
fn the_pinned_toolchain_passes_and_prints_the_exports() {
    let run = check(&Host::pinned());
    assert!(run.output.status.success(), "{}", run.stderr());
    let stdout = run.stdout();
    assert!(
        stdout.contains(&format!("export DEVELOPER_DIR=\"{}\"", run.developer_dir.display())),
        "{stdout}"
    );
    assert!(stdout.contains("export PATH=\""), "{stdout}");
    assert!(
        stdout.contains(":$PATH\""),
        "the PATH addition must keep $PATH:\n{stdout}"
    );
    assert_eq!(run.log, "", "--check changed something");
}

#[test]
fn another_xcode_fails() {
    let run = check(&Host {
        xcode: "26.6".into(),
        ..Host::pinned()
    });
    run.assert_fails_naming(&[&format!("Xcode {}", pin("IOS_XCODE_VERSION")), "Xcode 26.6"]);
    assert_eq!(run.log, "", "--check changed something");
}

#[test]
fn a_missing_rust_ios_target_fails() {
    let run = check(&Host {
        rust_targets: vec!["aarch64-apple-darwin", "aarch64-apple-ios"],
        ..Host::pinned()
    });
    run.assert_fails_naming(&["aarch64-apple-ios-sim"]);
    assert!(!run.stderr().contains("target aarch64-apple-ios "), "{}", run.stderr());
    assert_eq!(run.log, "", "--check changed something");
}

#[test]
fn a_missing_simulator_runtime_fails() {
    let run = check(&Host {
        runtimes: vec![runtime_line("26.5")],
        ..Host::pinned()
    });
    run.assert_fails_naming(&[&format!("iOS {} simulator runtime", pin("IOS_SIMULATOR_RUNTIME"))]);
    assert_eq!(run.log, "", "--check changed something");
}

#[test]
fn an_unavailable_simulator_runtime_fails() {
    let version = pin("IOS_SIMULATOR_RUNTIME");
    let run = check(&Host {
        runtimes: vec![format!(
            "{} (unavailable, runtime path not found)",
            runtime_line(&version)
        )],
        ..Host::pinned()
    });
    run.assert_fails_naming(&[&format!("iOS {version} simulator runtime")]);
}

#[test]
fn another_swiftlint_fails() {
    let run = check(&Host {
        swiftlint: Some("0.60.0".into()),
        ..Host::pinned()
    });
    run.assert_fails_naming(&[&format!("SwiftLint {}", pin("SWIFTLINT_VERSION")), "0.60.0"]);
}

#[test]
fn a_missing_swiftlint_fails() {
    let run = check(&Host {
        swiftlint: None,
        ..Host::pinned()
    });
    run.assert_fails_naming(&[&format!("SwiftLint {}", pin("SWIFTLINT_VERSION")), "not found"]);
}

#[test]
fn every_missing_piece_is_named_at_once() {
    let run = check(&Host {
        xcode: "26.6".into(),
        runtimes: Vec::new(),
        rust_targets: vec!["aarch64-apple-darwin"],
        swiftlint: None,
        ..Host::pinned()
    });
    run.assert_fails_naming(&[
        "Xcode 26.6",
        "aarch64-apple-ios",
        "aarch64-apple-ios-sim",
        "simulator runtime",
        "SwiftLint",
    ]);
    assert_eq!(run.log, "", "--check changed something");
}

#[test]
fn another_os_is_refused() {
    let run = check(&Host {
        os: "Linux",
        ..Host::pinned()
    });
    assert!(!run.output.status.success());
    assert!(run.stderr().contains("macOS"), "{}", run.stderr());
    assert_eq!(run.log, "");
}
