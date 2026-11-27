//! `scripts/ios-sim-runner.sh`, cargo's runner of the `aarch64-apple-ios-sim` target, and
//! `scripts/setup-ios.sh --simulator`, which the runner and `scripts/build-ios.sh` call right before each test run.
//!
//! `--simulator <name>` waits while a runtime disk image is in a transition, creates the simulator on the pinned runtime
//! again when it is gone, boots it, makes sure that simctl lists it as booted, disables and stops the simulator's
//! services that no test uses (a launchctl that refuses one leaves it running and the run goes on), and prints only its
//! UDID. The runner
//! then runs the executable on that phone through `xcrun simctl spawn` with its arguments, forwards `RUST_BACKTRACE`
//! and `RUST_TEST_*` (as `SIMCTL_CHILD_*`) and nothing else, and exits with the executable's status.
//!
//! Every case runs the real scripts with stub `uname`, `xcode-select`, `xcodebuild` and `xcrun` first on a `PATH` that
//! holds nothing else but `/usr/bin` and `/bin`, in a cleared environment, so nothing here needs a simulator. The stub
//! `xcrun` logs each call and keeps the simulators in a file (its `-f simctl` names a stub `simctl` that calls it
//! back): `simctl create` adds one, and `simctl bootstatus` marks one booted unless the case says the boot does not
//! take. Its `simctl spawn` of launchctl exits with the status the case asks for it, and any other `simctl spawn` prints
//! what the spawned process would get and exits with the status the case asks for. The
//! real runner on a real simulator is what `cargo test --target aarch64-apple-ios-sim` uses (Task 2 of the iOS plan).
//! Unix hosts only: the scripts are macOS-only, and the stub `uname` lets Linux run them too. They use the host's `jq`
//! in `/usr/bin`: macOS has it, and a Linux host needs it installed (the `unit-coverage` job of `linux.yml` installs
//! it).
#![cfg(unix)]

/// The stubs, the scratch directory and the runs of the scripts.
#[cfg(test)]
mod host {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use std::sync::mpsc;
    use std::time::Duration;

    pub(super) const UDID: &str = "0F6F99AC-7E30-476E-BCD5-248A5CD003BC";
    /// The UDID of a simulator that `simctl create` makes.
    pub(super) const NEW_UDID: &str = "A1B2C3D4-0000-4000-8000-00000000C0DE";
    /// Longer than one poll of the service wait (5 s), much shorter than its limit (600 s).
    const DEADLINE: Duration = Duration::from_secs(60);

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

    /// The identifier of the iOS runtime of `version`.
    pub(super) fn runtime_id(version: &str) -> String {
        format!("com.apple.CoreSimulator.SimRuntime.iOS-{}", version.replace('.', "-"))
    }

    /// The `xcrun simctl list runtimes` line of an available iOS runtime of `version`.
    pub(super) fn runtime_line(version: &str) -> String {
        format!("iOS {version} ({version} - 24A100) - {}", runtime_id(version))
    }

    /// A `simctl runtime list -j` answer: one disk image of each (runtime version, state).
    pub(super) fn images(entries: &[(&str, &str)]) -> String {
        let images: Vec<String> = entries
            .iter()
            .enumerate()
            .map(|(index, (version, state))| {
                format!(
                    r#""IMAGE-{index}": {{"runtimeIdentifier": "{}", "version": "{version}", "state": "{state}"}}"#,
                    runtime_id(version)
                )
            })
            .collect();
        format!("{{{}}}", images.join(", "))
    }

    fn write_stub(dir: &Path, name: &str, body: &str) {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// What the stubs report: the pinned phone on the pinned runtime, ready, unless a case changes one field.
    pub(super) struct World {
        /// The `simctl list devices` text at the start.
        pub(super) devices: String,
        /// The `simctl runtime list -j` answers, one for each call; the last one repeats.
        pub(super) images: Vec<String>,
        /// The `simctl list runtimes` lines.
        pub(super) runtimes: Vec<String>,
        /// Whether `simctl bootstatus -b` makes the simulator booted.
        pub(super) boots: bool,
        /// The exit status of `simctl spawn`.
        pub(super) status: i32,
        /// Whether `xcrun -f simctl` prints the path of simctl, as Xcode's xcrun does.
        pub(super) finds_simctl: bool,
        /// The exit status of `simctl spawn <udid> launchctl`.
        pub(super) launchctl_status: i32,
        /// The services that `launchctl print-disabled system` lists as disabled.
        pub(super) disabled: Vec<&'static str>,
    }

    impl World {
        pub(super) fn ready() -> Self {
            let version = pin("IOS_SIMULATOR_RUNTIME");
            Self {
                devices: devices(),
                images: vec![images(&[(version.as_str(), "Ready")])],
                runtimes: vec![runtime_line(&version)],
                boots: true,
                status: 0,
                finds_simctl: true,
                launchctl_status: 0,
                disabled: Vec::new(),
            }
        }
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

        /// The `xcrun` calls that change something: all but the `list` ones. The services are quieted side by side, so
        /// their `launchctl` calls are sorted where they came.
        pub(super) fn changes(&self) -> Vec<String> {
            let mut changes: Vec<String> = self
                .log
                .lines()
                .filter(|line| !line.starts_with("simctl list ") && !line.starts_with("simctl runtime list"))
                .map(str::to_owned)
                .collect();
            let places: Vec<usize> = (0..changes.len())
                .filter(|&index| is_launchctl(&changes[index]))
                .collect();
            let mut quieting: Vec<String> = places.iter().map(|&index| changes[index].clone()).collect();
            quieting.sort();
            for (index, line) in places.into_iter().zip(quieting) {
                changes[index] = line;
            }
            changes
        }
    }

    /// The xcrun stub: it logs each call and keeps the simulators in `$STUB_DIR/devices`. Its `-f simctl` names the
    /// simctl stub in the developer directory, which hands its call back to it as `xcrun simctl`.
    fn xcrun_stub(world: &World) -> String {
        let find = if world.finds_simctl {
            r#"echo "$STUB_DEVELOPER_DIR/usr/bin/simctl""#
        } else {
            ":"
        };
        let header = format!("-- iOS {} --", pin("IOS_SIMULATOR_RUNTIME"));
        let boot = if world.boots {
            r#"sed "s/($3) (Shutdown)/($3) (Booted)/" "$STUB_DIR/devices" >"$STUB_DIR/devices.new"
  mv "$STUB_DIR/devices.new" "$STUB_DIR/devices""#
        } else {
            ":"
        };
        format!(
            r#"if [ "$1 $2" = '-f simctl' ]; then
  {find}
  exit 0
fi
echo "$*" >>"$STUB_DIR/xcrun.log"
case "$1 $2 $3" in
  'simctl runtime list')
    n=$(($(cat "$STUB_DIR/image-calls") + 1))
    echo "$n" >"$STUB_DIR/image-calls"
    if [ -f "$STUB_DIR/images-$n.json" ]; then cat "$STUB_DIR/images-$n.json"; else cat "$STUB_DIR/images-last.json"; fi
    exit 0 ;;
  'simctl list runtimes') printf '== Runtimes ==\n'; cat "$STUB_DIR/runtimes"; exit 0 ;;
  'simctl list devices') cat "$STUB_DIR/devices"; exit 0 ;;
esac
case "$1 $2" in
  'simctl create') printf '%s\n    %s ({NEW_UDID}) (Shutdown)\n' '{header}' "$3" >>"$STUB_DIR/devices"; echo {NEW_UDID}; exit 0 ;;
  'simctl bootstatus')
  {boot}
    exit 0 ;;
  'simctl spawn')
    if [ "$4 $5" = 'launchctl print-disabled' ]; then cat "$STUB_DIR/disabled"; fi
    if [ "$4" = launchctl ]; then exit {}; fi
    env | grep '^SIMCTL_CHILD_' | sort; exit {} ;;
esac
exit 0
"#,
            world.launchctl_status, world.status
        )
    }

    /// `script` (relative to scripts/) with `args`, run from a scratch directory over `world`, with `env` set. The run
    /// fails the case when it takes longer than DEADLINE.
    pub(super) fn run_script(world: &World, script: &str, args: &[&str], env: &[(&str, &str)]) -> Run {
        let scratch = tempfile::tempdir().unwrap();
        let stubs = scratch.path().join("stubs");
        let developer_dir = scratch.path().join("Xcode.app/Contents/Developer");
        fs::create_dir_all(&stubs).unwrap();
        fs::create_dir_all(&developer_dir).unwrap();
        fs::create_dir_all(scratch.path().join("deps")).unwrap();
        fs::write(scratch.path().join("deps/tests"), "").unwrap();
        fs::write(scratch.path().join("devices"), &world.devices).unwrap();
        fs::write(scratch.path().join("runtimes"), world.runtimes.join("\n") + "\n").unwrap();
        fs::write(scratch.path().join("image-calls"), "0\n").unwrap();
        for (index, answer) in world.images.iter().enumerate() {
            fs::write(scratch.path().join(format!("images-{}.json", index + 1)), answer).unwrap();
        }
        fs::write(scratch.path().join("images-last.json"), world.images.last().unwrap()).unwrap();
        fs::write(scratch.path().join("xcrun.log"), "").unwrap();
        let disabled = world.disabled.iter().fold(String::new(), |listing, label| {
            listing + "\t\t\"" + label + "\" => disabled\n"
        });
        fs::write(
            scratch.path().join("disabled"),
            format!("\tdisabled services = {{\n{disabled}\t}}\n"),
        )
        .unwrap();
        write_stub(
            &stubs,
            "uname",
            "case \"$1\" in -m) echo arm64 ;; *) echo Darwin ;; esac\n",
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
                "[ \"$1\" = -version ] && printf 'Xcode %s\\nBuild version 27B52\\n' {}\n",
                pin("IOS_XCODE_VERSION")
            ),
        );
        write_stub(&stubs, "xcrun", &xcrun_stub(world));
        let simctl_dir = developer_dir.join("usr/bin");
        fs::create_dir_all(&simctl_dir).unwrap();
        write_stub(&simctl_dir, "simctl", "exec \"$STUB_DIR/stubs/xcrun\" simctl \"$@\"\n");
        let mut command = Command::new("bash");
        command
            .arg(repo_root().join("scripts").join(script))
            .args(args)
            .current_dir(scratch.path())
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", stubs.display()))
            .env("HOME", scratch.path())
            .env("TMPDIR", scratch.path())
            .env("NGTERM_VENDOR", scratch.path().join("vendor"))
            .env("STUB_DIR", scratch.path())
            .env("STUB_DEVELOPER_DIR", &developer_dir);
        for (name, value) in env {
            command.env(name, value);
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || sender.send(command.output()));
        let output = receiver
            .recv_timeout(DEADLINE)
            .unwrap_or_else(|_| panic!("scripts/{script} {args:?} ran for more than {DEADLINE:?}"))
            .unwrap();
        // the runner's $PWD is the physical directory (the environment has no PWD), and macOS's temporary directory
        // is behind a symlink
        let executable = fs::canonicalize(scratch.path())
            .unwrap()
            .join(args.first().unwrap_or(&""));
        Run {
            output,
            log: fs::read_to_string(scratch.path().join("xcrun.log")).unwrap(),
            executable,
        }
    }

    /// The runner on `executable` (relative to the scratch directory) and `args`, over `world`, with `env` set.
    pub(super) fn runner(world: &World, executable: &str, args: &[&str], env: &[(&str, &str)]) -> Run {
        let all: Vec<&str> = std::iter::once(executable).chain(args.iter().copied()).collect();
        run_script(world, "ios-sim-runner.sh", &all, env)
    }

    pub(super) fn is_launchctl(call: &str) -> bool {
        call.starts_with("simctl spawn ") && call.contains(" launchctl ")
    }

    /// The services that `--simulator` disables and stops in simulator `udid`: those whose processes took a CI Mac's
    /// free memory to 25% mid-run (nightly run 45820517849).
    pub(super) const QUIET: [&str; 8] = [
        "com.apple.chronod",
        "com.apple.healthappd",
        "com.apple.siriactionsd",
        "com.apple.intelligenceflowd",
        "com.apple.intelligencecontextd",
        "com.apple.sirittsd",
        "com.apple.photoanalysisd",
        "com.apple.newsd",
    ];

    /// The launchctl calls that quiet simulator `udid` when none of its services is disabled yet, as the stub xcrun
    /// logs them (sorted): the read of what is disabled, then a disable and a stop for each service.
    pub(super) fn quieting(udid: &str) -> Vec<String> {
        QUIET
            .iter()
            .flat_map(|label| {
                ["disable", "bootout"].map(|verb| format!("simctl spawn {udid} launchctl {verb} system/{label}"))
            })
            .chain([format!("simctl spawn {udid} launchctl print-disabled system")])
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// `scripts/setup-ios.sh --simulator <name>` over `world`.
    pub(super) fn simulator(world: &World, name: &str) -> Run {
        run_script(world, "setup-ios.sh", &["--simulator", name], &[])
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

use host::{
    NEW_UDID, QUIET, UDID, World, images, is_launchctl, pin, quieting, repo_root, runner, runtime_id, runtime_line,
    simulator,
};

#[test]
fn it_runs_the_executable_on_the_pinned_phone_and_forwards_only_rusts_test_variables() {
    let run = runner(
        &World::ready(),
        "deps/tests",
        &["--exact", "a test"],
        &[
            ("RUST_BACKTRACE", "1"),
            ("RUST_TEST_THREADS", "3"),
            ("RUST_TEST_NOCAPTURE", "1"),
            ("RUST_LOG", "debug"),
            ("OTHER", "x"),
        ],
    );
    assert!(run.output.status.success(), "{}", run.stderr());
    assert_eq!(
        run.changes(),
        [
            vec![format!("simctl bootstatus {UDID} -b")],
            quieting(UDID),
            vec![format!(
                "simctl spawn {UDID} {} --exact a test",
                run.executable.display()
            )],
        ]
        .concat(),
        "boots the pinned runtime's simulator, quiets it, then spawns the executable on it by absolute path"
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
        let run = runner(
            &World {
                status,
                ..World::ready()
            },
            "deps/tests",
            &[],
            &[],
        );
        assert_eq!(run.output.status.code(), Some(status), "{}", run.stderr());
    }
}

#[test]
fn a_missing_simulator_is_created_again_and_the_executable_runs_on_it() {
    let elsewhere = format!(
        "== Devices ==\n-- iOS 18.0 --\n    {} ({UDID}) (Shutdown)\n",
        pin("IOS_SIM_PHONE_NAME")
    );
    let run = runner(
        &World {
            devices: elsewhere,
            ..World::ready()
        },
        "deps/tests",
        &[],
        &[],
    );
    assert!(run.output.status.success(), "{}", run.stderr());
    assert_eq!(
        run.changes(),
        [
            vec![
                format!(
                    "simctl create {} {} {}",
                    pin("IOS_SIM_PHONE_NAME"),
                    pin("IOS_SIM_PHONE"),
                    runtime_id(&pin("IOS_SIMULATOR_RUNTIME"))
                ),
                format!("simctl bootstatus {NEW_UDID} -b"),
            ],
            quieting(NEW_UDID),
            vec![format!("simctl spawn {NEW_UDID} {}", run.executable.display())],
        ]
        .concat()
    );
}

#[test]
fn a_simulator_that_is_not_booted_after_its_boot_spawns_nothing() {
    let run = runner(
        &World {
            boots: false,
            ..World::ready()
        },
        "deps/tests",
        &[],
        &[],
    );
    assert!(!run.output.status.success());
    let stderr = run.stderr();
    assert!(stderr.contains("is not booted after its boot"), "{stderr}");
    assert!(stderr.contains("is not ready"), "{stderr}");
    assert!(!run.log.contains("spawn"), "{}", run.log);
}

#[test]
fn a_simctl_that_xcrun_does_not_find_boots_and_spawns_nothing() {
    let run = runner(
        &World {
            finds_simctl: false,
            ..World::ready()
        },
        "deps/tests",
        &[],
        &[],
    );
    assert!(!run.output.status.success());
    let stderr = run.stderr();
    assert!(stderr.contains("xcrun -f simctl gave no simctl to run"), "{stderr}");
    assert!(!stderr.contains("command not found"), "{stderr}");
    assert_eq!(run.changes(), Vec::<String>::new(), "{stderr}");
}

#[test]
fn a_missing_executable_or_no_arguments_fails_before_the_simulator() {
    let missing = runner(&World::ready(), "deps/absent", &[], &[]);
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

#[test]
fn simulator_prints_only_the_udid_of_the_booted_simulator() {
    let tablet = pin("IOS_SIM_TABLET_NAME");
    let world = World {
        devices: format!("{}    {tablet} ({NEW_UDID}) (Booted)\n", host::devices()),
        ..World::ready()
    };
    let phone = simulator(&world, &pin("IOS_SIM_PHONE_NAME"));
    assert!(phone.output.status.success(), "{}", phone.stderr());
    assert_eq!(
        phone.stdout(),
        format!("{UDID}\n"),
        "build-ios.sh puts it in -destination id="
    );
    let tablet = simulator(&world, &tablet);
    assert!(tablet.output.status.success(), "{}", tablet.stderr());
    assert_eq!(tablet.stdout(), format!("{NEW_UDID}\n"));
    assert_eq!(
        tablet.changes(),
        [vec![format!("simctl bootstatus {NEW_UDID} -b")], quieting(NEW_UDID)].concat()
    );
}

#[test]
fn simulator_says_which_services_it_disabled() {
    let run = simulator(&World::ready(), &pin("IOS_SIM_PHONE_NAME"));
    assert!(run.output.status.success(), "{}", run.stderr());
    let stderr = run.stderr();
    assert!(
        stderr.contains(&format!(
            "({UDID}): disabled com.apple.chronod com.apple.healthappd com.apple.siriactionsd \
             com.apple.intelligenceflowd com.apple.intelligencecontextd com.apple.sirittsd com.apple.photoanalysisd \
             com.apple.newsd of 8 services no test uses"
        )),
        "{stderr}"
    );
}

#[test]
fn simulator_disables_each_service_before_it_stops_it() {
    let run = simulator(&World::ready(), &pin("IOS_SIM_PHONE_NAME"));
    assert!(run.output.status.success(), "{}", run.stderr());
    let calls: Vec<&str> = run.log.lines().collect();
    let place = |call: &str| calls.iter().position(|line| *line == call);
    for bootout in quieting(UDID).iter().filter(|call| call.contains(" bootout ")) {
        let disable = bootout.replace(" bootout ", " disable ");
        assert!(
            place(&disable).unwrap() < place(bootout).unwrap(),
            "a stopped service would start again: {}",
            run.log
        );
    }
    let boot = place(&format!("simctl bootstatus {UDID} -b")).unwrap();
    let first = calls.iter().position(|line| is_launchctl(line)).unwrap();
    assert!(boot < first, "the services are quieted after the boot: {}", run.log);
}

#[test]
fn simulator_leaves_the_services_already_disabled_as_they_are() {
    let run = simulator(
        &World {
            disabled: QUIET[..6].to_vec(),
            ..World::ready()
        },
        &pin("IOS_SIM_PHONE_NAME"),
    );
    assert!(run.output.status.success(), "{}", run.stderr());
    let quieted: Vec<String> = run.changes().into_iter().filter(|call| is_launchctl(call)).collect();
    let rest = QUIET[6..].iter().flat_map(|label| {
        ["disable", "bootout"].map(|verb| format!("simctl spawn {UDID} launchctl {verb} system/{label}"))
    });
    let mut expected: Vec<String> = rest
        .chain([format!("simctl spawn {UDID} launchctl print-disabled system")])
        .collect();
    expected.sort();
    assert_eq!(
        quieted, expected,
        "the runner readies its simulator once per test binary"
    );
    assert!(
        run.stderr()
            .contains(&format!("({UDID}): disabled {} of 8", QUIET.join(" "))),
        "{}",
        run.stderr()
    );
}

#[test]
fn a_service_that_launchctl_refuses_leaves_the_simulator_ready() {
    let run = simulator(
        &World {
            launchctl_status: 113,
            ..World::ready()
        },
        &pin("IOS_SIM_PHONE_NAME"),
    );
    assert!(run.output.status.success(), "{}", run.stderr());
    assert_eq!(run.stdout(), format!("{UDID}\n"));
    assert!(
        run.stderr().contains(&format!("({UDID}): disabled none of 8")),
        "{}",
        run.stderr()
    );
    assert!(
        !run.log.contains("launchctl bootout"),
        "a service it could not disable is not stopped: {}",
        run.log
    );
}

#[test]
fn simulator_refuses_an_unknown_name() {
    let run = simulator(&World::ready(), "ngterm-watch");
    assert!(!run.output.status.success());
    let stderr = run.stderr();
    assert!(stderr.contains(&pin("IOS_SIM_PHONE_NAME")), "{stderr}");
    assert!(stderr.contains(&pin("IOS_SIM_TABLET_NAME")), "{stderr}");
    assert_eq!(run.log, "");
    assert_eq!(run.stdout(), "");
}

#[test]
fn simulator_waits_while_a_runtime_disk_image_mounts() {
    let version = pin("IOS_SIMULATOR_RUNTIME");
    let run = simulator(
        &World {
            images: vec![
                images(&[(version.as_str(), "Mounting")]),
                images(&[(version.as_str(), "Ready")]),
            ],
            ..World::ready()
        },
        &pin("IOS_SIM_PHONE_NAME"),
    );
    assert!(run.output.status.success(), "{}", run.stderr());
    let stderr = run.stderr();
    assert!(stderr.contains("waiting for the simulator service"), "{stderr}");
    assert!(stderr.contains("Mounting"), "{stderr}");
    assert_eq!(run.stdout(), format!("{UDID}\n"));
}

#[test]
fn simulator_does_not_wait_for_images_that_stay_unusable() {
    let version = pin("IOS_SIMULATOR_RUNTIME");
    let run = simulator(
        &World {
            images: vec![images(&[
                ("26.5", "Unusable - Other Failure: Duplicate of IMAGE-1"),
                ("26.4", "Deleting"),
                ("26.3", "Unknown"),
                (version.as_str(), "Ready"),
            ])],
            ..World::ready()
        },
        &pin("IOS_SIM_PHONE_NAME"),
    );
    assert!(run.output.status.success(), "{}", run.stderr());
    assert!(!run.stderr().contains("waiting"), "{}", run.stderr());
    assert_eq!(run.stdout(), format!("{UDID}\n"));
}

#[test]
fn simulator_fails_at_once_on_an_unavailable_runtime() {
    let version = pin("IOS_SIMULATOR_RUNTIME");
    let run = simulator(
        &World {
            runtimes: vec![format!(
                "{} (unavailable, runtime profile not found)",
                runtime_line(&version)
            )],
            ..World::ready()
        },
        &pin("IOS_SIM_PHONE_NAME"),
    );
    assert!(!run.output.status.success());
    let stderr = run.stderr();
    assert!(stderr.contains("run scripts/setup-ios.sh"), "{stderr}");
    assert!(
        !stderr.contains("waiting"),
        "a lasting fault is not waited for:\n{stderr}"
    );
    assert_eq!(run.changes(), Vec::<String>::new());
    assert_eq!(run.stdout(), "");
}
