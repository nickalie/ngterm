//! Ported from upstream's `TerminfoInstallTests` and `TerminfoCommandsTests`: a fake `ssh` in a temp root (an `sh`
//! script, on Windows a `.cmd` over PowerShell), compiled entries written byte by byte (the real `tic` only against a
//! temp database), and the remote side as a temp `HOME`. Upstream's fake `infocmp` has no counterpart: the dump is
//! this build's own decompiler ([`source`]), tested against the real `infocmp` in its own tests.

use std::fs;
#[cfg(unix)]
use std::process::Command as Process;

use clap::Parser;

use super::source::tests::Compiled;
#[cfg(unix)]
use super::source::tests::Slot;
use super::*;
use crate::{Command as Cli, Ngtermctl};

/// A compiled `xterm-ghostty` entry named `names`, with `cols#80`.
fn compiled_entry(names: &str) -> Vec<u8> {
    Compiled {
        names,
        booleans: &[],
        numbers: &[80],
        strings: &[],
        extended: &[],
        wide: false,
    }
    .bytes()
}

/// What the dump of [`compiled_entry`] sends, read from `file`.
fn dumped(names: &str, file: &Path) -> String {
    format!(
        "#\tReconstructed via ngtermctl from file: {}\n{names},\n\tcols#80,\n",
        file.display()
    )
}

fn env(pairs: &[(&str, &Path)]) -> HashMap<OsString, OsString> {
    pairs
        .iter()
        .map(|(name, value)| (OsString::from(name), value.as_os_str().to_owned()))
        .collect()
}

/// A temp root holding a fake `ssh`, a fake stage, and what the fake recorded.
struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    #[cfg(unix)]
    fn script(path: &Path, body: &str) -> PathBuf {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        ngterm_host::fakes::write_executable(path, &format!("#!/bin/sh\n{body}\n")).unwrap();
        path.to_path_buf()
    }

    /// `<root>/<name>/<layout>/xterm-ghostty`, holding `bytes`.
    fn database_with(&self, name: &str, layout: &str, bytes: &[u8]) -> PathBuf {
        let dir = self.path(name);
        fs::create_dir_all(dir.join(layout)).unwrap();
        fs::write(dir.join(layout).join(ENTRY), bytes).unwrap();
        dir
    }

    /// `<root>/<name>/<layout>/xterm-ghostty`, a compiled entry named `xterm-ghostty|<name>`.
    fn database(&self, name: &str, layout: &str) -> PathBuf {
        self.database_with(name, layout, &compiled_entry(&format!("{ENTRY}|{name}")))
    }

    /// The staging layout the installed CLI resolves into: `bin/ngtermctl` beside `share/terminfo`.
    fn stage(&self, layout: &str) -> (PathBuf, PathBuf) {
        let bin = self.path("stage/bin");
        fs::create_dir_all(&bin).unwrap();
        let client = bin.join("ngtermctl");
        fs::write(&client, b"").unwrap();
        (client, self.database("stage/share/terminfo", layout))
    }

    /// A database the real `tic` compiled from `source`.
    #[cfg(unix)]
    fn compiled_database(&self, source: &str) -> PathBuf {
        let dir = self.path("compiled");
        fs::create_dir_all(&dir).unwrap();
        let mut tic = Process::new("/usr/bin/tic")
            .args(["-x", "-o"])
            .arg(&dir)
            .arg("-")
            .stdin(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        tic.stdin.take().unwrap().write_all(source.as_bytes()).unwrap();
        assert!(tic.wait().unwrap().success(), "tic could not compile the fixture entry");
        dir
    }

    /// Records its arguments one per line and, unless told not to, copies stdin to a file.
    #[cfg(unix)]
    fn fake_ssh(&self, exit_code: i32, reads_stdin: bool, extra: &str) -> String {
        let read = if reads_stdin {
            format!("cat > '{}'", self.path("ssh.stdin").display())
        } else {
            String::new()
        };
        let script = Self::script(
            &self.path("ssh"),
            &format!(
                "printf '%s\\n' \"$@\" > '{}'\n{extra}\n{read}\nexit {exit_code}",
                self.path("ssh.args").display()
            ),
        );
        script.display().to_string()
    }

    /// A `.cmd` fake over PowerShell that records its arguments one per line, copies stdin byte for byte to a file
    /// and exits `exit_code`.
    #[cfg(windows)]
    fn fake_ssh(&self, exit_code: i32) -> String {
        let quoted = |name: &str| format!("'{}'", self.path(name).display());
        let script = self.path("ssh.ps1");
        fs::write(
            &script,
            format!(
                "$args | Set-Content -Path {args}\r\n\
                 $out = [IO.File]::Create({stdin}); [Console]::OpenStandardInput().CopyTo($out); $out.Close()\r\n\
                 exit {exit_code}\r\n",
                args = quoted("ssh.args"),
                stdin = quoted("ssh.stdin"),
            ),
        )
        .unwrap();
        let executable = self.path("ssh.cmd");
        fs::write(
            &executable,
            format!(
                "@powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{}\" %*\r\n\
                 @exit /b %ERRORLEVEL%\r\n",
                script.display()
            ),
        )
        .unwrap();
        executable.display().to_string()
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.path(name)).unwrap()
    }

    fn ssh_arguments(&self) -> Vec<String> {
        self.read("ssh.args").lines().map(str::to_owned).collect()
    }

    /// A stand-in for the remote account: an empty `HOME` and a `PATH` of a bin dir plus `/bin` (mkdir, but on a
    /// merged-usr system tic too, so the no-tic case uses the bin dir alone) and a fake `tic` that records its
    /// arguments and stdin, or none.
    #[cfg(unix)]
    fn remote_home(&self, with_tic: bool) -> (PathBuf, Vec<(&'static str, String)>) {
        let home = self.path("remote-home");
        let bin = self.path("remote-bin");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let path = if with_tic {
            Self::script(
                &bin.join("tic"),
                &format!(
                    "printf '%s\\n' \"$@\" > '{}'\ncat > '{}'",
                    self.path("tic.args").display(),
                    self.path("tic.stdin").display()
                ),
            );
            format!("{}:/bin", bin.display())
        } else {
            for tool in ["mkdir", "printf"] {
                std::os::unix::fs::symlink(Path::new("/usr/bin").join(tool), bin.join(tool)).unwrap();
            }
            bin.display().to_string()
        };
        (home.clone(), vec![("HOME", home.display().to_string()), ("PATH", path)])
    }
}

fn connection(destination: &str) -> Connection {
    Connection {
        destination: destination.to_owned(),
        ..Connection::default()
    }
}

// -- the command line

fn parse(argv: &[&str]) -> Result<Install, clap::Error> {
    let line = ["ngtermctl", "terminfo", "install"].iter().chain(argv).copied();
    let Cli::Terminfo(group) = Ngtermctl::try_parse_from(line)?.command else {
        panic!("not a terminfo command")
    };
    let TerminfoCommand::Install(install) = group.command;
    Ok(install)
}

#[test]
fn install_maps_its_typed_options_onto_the_connection() {
    let install = parse(&[
        "me@buildbox",
        "-p",
        "2222",
        "-i",
        "/k1",
        "-i",
        "/k2",
        "-J",
        "bastion",
        "-F",
        "/cfg",
    ])
    .unwrap();
    assert_eq!(
        install.connection(),
        Connection {
            destination: "me@buildbox".to_owned(),
            port: Some(2222),
            identities: vec!["/k1".to_owned(), "/k2".to_owned()],
            jump: Some("bastion".to_owned()),
            config: Some("/cfg".to_owned()),
        }
    );
    assert_eq!(parse(&["buildbox"]).unwrap().connection(), connection("buildbox"));
}

#[test]
fn install_refuses_anything_beyond_the_destination_and_its_options() {
    for argv in [
        &[][..],
        &["buildbox", "uptime"],
        &["--", "-G"],
        &["-N", "buildbox"],
        &["buildbox", "--socket", "/tmp/s"],
    ] {
        let refused = parse(argv)
            .map_err(drop)
            .and_then(|install| install.validate().map_err(drop));
        assert!(refused.is_err(), "{argv:?}");
    }
}

#[test]
fn install_validation_names_the_destination() {
    for destination in ["-oProxyCommand=x", "two words", ""] {
        assert_eq!(
            parse(&["--", destination]).unwrap().validate(),
            Err(format!(
                "destination must be a host, user@host or an ssh alias, not '{destination}'"
            ))
        );
    }
}

#[test]
fn every_failure_has_its_own_message() {
    let cases = [
        (
            InstallFailure::TerminfoNotFound {
                searched: vec![PathBuf::from("/a/share/terminfo"), PathBuf::from("/t")],
            },
            "no xterm-ghostty terminfo entry next to this ngtermctl; looked in /a/share/terminfo, /t",
        ),
        (
            InstallFailure::EntryUnreadable {
                entry: PathBuf::from("/db/x/xterm-ghostty"),
                reason: "the compiled entry is truncated".to_owned(),
            },
            "could not read /db/x/xterm-ghostty: the compiled entry is truncated",
        ),
        (
            InstallFailure::SpawnFailed {
                operation: "posix_spawnp".to_owned(),
                reason: "No such file or directory".to_owned(),
            },
            "could not start ssh: posix_spawnp failed with No such file or directory",
        ),
    ];
    for (failure, message) in cases {
        assert_eq!(failure.message(), message);
    }
}

// -- where the entry comes from

#[test]
fn the_stage_beside_the_client_wins_over_the_environment() {
    let fixture = Fixture::new();
    let (client, terminfo) = fixture.stage("78");
    let from_env = fixture.database("env", "x");
    assert_eq!(
        terminfo_directory(Some(&client), &env(&[("TERMINFO", &from_env)])),
        Some(terminfo)
    );
}

#[test]
fn the_environment_is_the_fallback_when_the_client_has_no_stage() {
    let fixture = Fixture::new();
    let from_env = fixture.database("env", "x");
    let loose = fixture.path("target/debug/ngtermctl");
    let environment = env(&[("TERMINFO", &from_env)]);
    assert_eq!(terminfo_directory(Some(&loose), &environment), Some(from_env.clone()));
    assert_eq!(terminfo_directory(None, &environment), Some(from_env));
}

#[test]
fn a_directory_without_the_entry_is_skipped() {
    let fixture = Fixture::new();
    let empty = fixture.path("empty");
    fs::create_dir_all(&empty).unwrap();
    assert_eq!(terminfo_directory(None, &env(&[("TERMINFO", &empty)])), None);
    assert_eq!(terminfo_directory(None, &HashMap::new()), None);
    assert!(candidate_directories(None, &env(&[("TERMINFO", Path::new(""))])).is_empty());
}

// -- the ssh invocation

#[test]
fn install_maps_every_connection_option_after_the_execution_settings() {
    let argv = install_command(
        &Connection {
            destination: "me@buildbox".to_owned(),
            port: Some(2222),
            identities: vec!["/k1".to_owned(), "/k2".to_owned()],
            jump: Some("bastion".to_owned()),
            config: Some("/cfg".to_owned()),
        },
        "ssh",
    )
    .unwrap();
    assert_eq!(argv[0], "ssh");
    assert_eq!(argv[1..=EXECUTION_OPTIONS.len()], EXECUTION_OPTIONS);
    assert_eq!(
        argv[1 + EXECUTION_OPTIONS.len()..argv.len() - 1],
        [
            "-p",
            "2222",
            "-i",
            "/k1",
            "-i",
            "/k2",
            "-J",
            "bastion",
            "-F",
            "/cfg",
            "me@buildbox"
        ]
    );
    let bare = install_command(&connection("buildbox"), "ssh").unwrap();
    assert_eq!(bare.len(), EXECUTION_OPTIONS.len() + 3);
    assert_eq!(bare[EXECUTION_OPTIONS.len() + 1], "buildbox");
}

#[test]
#[cfg(unix)]
fn install_overrides_the_execution_settings_a_config_file_sets() {
    let ssh = Path::new("/usr/bin/ssh");
    if !ssh.exists() {
        return;
    }
    let fixture = Fixture::new();
    let hostile = fixture.path("hostile.sshconfig");
    fs::write(
        &hostile,
        "Host *\n  RequestTTY yes\n  StdinNull yes\n  SessionType none\n  ForkAfterAuthentication yes\n  RemoteCommand \
         tmux attach\n",
    )
    .unwrap();
    let mut with_config = connection("example.invalid");
    with_config.config = Some(hostile.display().to_string());
    let argv = install_command(&with_config, "/usr/bin/ssh").unwrap();
    let resolved = Process::new(ssh).arg("-G").args(&argv[1..]).output().unwrap();
    assert!(
        resolved.status.success(),
        "{}",
        String::from_utf8_lossy(&resolved.stderr)
    );
    let settings: HashMap<String, String> = String::from_utf8_lossy(&resolved.stdout)
        .lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    assert_eq!(settings.get("requesttty").map(String::as_str), Some("false"));
    assert_eq!(settings.get("stdinnull").map(String::as_str), Some("no"));
    assert_eq!(settings.get("sessiontype").map(String::as_str), Some("default"));
    assert_eq!(settings.get("forkafterauthentication").map(String::as_str), Some("no"));
    assert_eq!(settings.get("remotecommand"), None);
}

#[test]
#[cfg(unix)]
fn the_remote_command_is_one_quoted_sh_invocation_on_one_line() {
    let argv = install_command(&connection("buildbox"), "ssh").unwrap();
    let remote = argv.last().unwrap();
    assert!(remote.starts_with("'/bin/sh' '-c' '"), "{remote}");
    assert!(!remote.contains('\n'));
}

/// The installed login shells among those upstream runs the remote command under.
#[cfg(unix)]
fn login_shells() -> Vec<&'static str> {
    ["/bin/sh", "/bin/bash", "/bin/zsh", "/bin/dash", "/bin/tcsh", "/bin/csh"]
        .into_iter()
        .filter(|shell| Path::new(shell).exists())
        .collect()
}

#[test]
#[cfg(unix)]
fn the_remote_command_compiles_the_source_under_every_login_shell() {
    let remote = install_command(&connection("buildbox"), "ssh").unwrap().pop().unwrap();
    for shell in login_shells() {
        let fixture = Fixture::new();
        let (home, environment) = fixture.remote_home(true);
        let mut child = Process::new(shell)
            .args(["-c", &remote])
            .env_clear()
            .envs(environment)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"xterm-ghostty|test,\n").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let terminfo = home.join(".terminfo");
        assert_eq!(
            fixture.read("tic.args").lines().collect::<Vec<_>>(),
            ["-x", "-o", &terminfo.display().to_string(), "-"],
            "{shell}"
        );
        assert_eq!(fixture.read("tic.stdin"), "xterm-ghostty|test,\n");
        assert!(terminfo.is_dir());
    }
}

#[test]
#[cfg(unix)]
fn the_remote_command_says_so_and_exits_3_when_the_host_has_no_tic() {
    let remote = install_command(&connection("buildbox"), "ssh").unwrap().pop().unwrap();
    for shell in login_shells() {
        let fixture = Fixture::new();
        let (home, environment) = fixture.remote_home(false);
        let output = Process::new(shell)
            .args(["-c", &remote])
            .env_clear()
            .envs(environment)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3), "{shell}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("tic is not installed on this host"));
        assert!(!home.join(".terminfo").exists());
    }
}

#[test]
fn an_option_looking_or_unprintable_destination_is_refused() {
    for destination in ["", "-G", "-N", "host name", "host\tname", "\u{1b}host", "host\u{7f}"] {
        assert_eq!(
            install_command(&connection(destination), "ssh"),
            Err(InstallFailure::InvalidDestination),
            "{destination:?}"
        );
    }
}

// -- the pipeline, with fake executables

#[test]
#[cfg(unix)]
fn the_dump_reaches_ssh_stdin_byte_for_byte_and_ends_in_eof() {
    let fixture = Fixture::new();
    let database = fixture.database("env", "78");
    let ssh = fixture.fake_ssh(0, true, "");
    let outcome = run(&connection("buildbox"), None, &env(&[("TERMINFO", &database)]), &ssh);
    assert_eq!(outcome, Ok(Outcome::Exited(0)));
    let file = database.join("78").join(ENTRY);
    assert_eq!(fixture.read("ssh.stdin"), dumped("xterm-ghostty|env", &file));
    let arguments = fixture.ssh_arguments();
    assert_eq!(arguments[0], "-T");
    assert!(arguments.contains(&"buildbox".to_owned()));
}

#[test]
#[cfg(windows)]
fn the_dump_reaches_ssh_stdin_byte_for_byte_and_ends_in_eof() {
    let fixture = Fixture::new();
    let database = fixture.database("env", "78");
    let ssh = fixture.fake_ssh(0);
    let outcome = run(&connection("buildbox"), None, &env(&[("TERMINFO", &database)]), &ssh);
    assert_eq!(outcome, Ok(Outcome::Exited(0)));
    let file = database.join("78").join(ENTRY);
    assert_eq!(fixture.read("ssh.stdin"), dumped("xterm-ghostty|env", &file));
    let arguments = fixture.ssh_arguments();
    assert_eq!(arguments[0], "-T");
    assert!(arguments.contains(&"buildbox".to_owned()));
}

#[test]
fn the_dump_is_the_entry_of_the_chosen_database_alone() {
    let fixture = Fixture::new();
    let (client, terminfo) = fixture.stage("x");
    fixture.database_with("env", "x", b"not an entry");
    let entry = terminfo.join("x").join(ENTRY);
    assert_eq!(
        dump(&terminfo).map(String::from_utf8),
        Ok(Ok(dumped("xterm-ghostty|stage/share/terminfo", &entry)))
    );
    assert_eq!(
        terminfo_directory(Some(&client), &env(&[("TERMINFO", &fixture.path("env"))])),
        Some(terminfo)
    );
}

#[test]
fn an_unreadable_entry_names_its_file_and_why() {
    let fixture = Fixture::new();
    let database = fixture.database_with("env", "78", b"xterm-ghostty|test,\n\tcols#80,\n");
    assert_eq!(
        dump(&database),
        Err(InstallFailure::EntryUnreadable {
            entry: database.join("78").join(ENTRY),
            reason: "not a compiled terminfo entry".to_owned()
        })
    );
    let truncated = compiled_entry("xterm-ghostty|test");
    let database = fixture.database_with("short", "x", &truncated[..truncated.len() - 1]);
    assert_eq!(
        dump(&database).map_err(|failure| failure.message()),
        Err(format!(
            "could not read {}: the compiled entry is truncated",
            database.join("x").join(ENTRY).display()
        ))
    );
}

#[test]
#[cfg(unix)]
fn a_failed_dump_never_starts_ssh() {
    let fixture = Fixture::new();
    let database = fixture.database_with("env", "78", b"");
    let ssh = fixture.fake_ssh(0, true, "");
    let outcome = run(&connection("buildbox"), None, &env(&[("TERMINFO", &database)]), &ssh);
    assert_eq!(
        outcome,
        Err(InstallFailure::EntryUnreadable {
            entry: database.join("78").join(ENTRY),
            reason: "not a compiled terminfo entry".to_owned()
        })
    );
    assert!(!fixture.path("ssh.args").exists());
}

#[test]
#[cfg(unix)]
fn a_missing_entry_never_starts_anything() {
    let fixture = Fixture::new();
    let ssh = fixture.fake_ssh(0, true, "");
    let outcome = run(&connection("buildbox"), None, &HashMap::new(), &ssh);
    assert_eq!(outcome, Err(InstallFailure::TerminfoNotFound { searched: vec![] }));
    assert!(!fixture.path("ssh.args").exists());
}

#[test]
#[cfg(unix)]
fn a_non_zero_remote_status_is_reported_as_itself() {
    let fixture = Fixture::new();
    let database = fixture.database("env", "78");
    let ssh = fixture.fake_ssh(3, true, "");
    let outcome = run(&connection("buildbox"), None, &env(&[("TERMINFO", &database)]), &ssh);
    assert_eq!(outcome, Ok(Outcome::Exited(3)));
}

#[test]
#[cfg(windows)]
fn a_non_zero_remote_status_is_reported_as_itself() {
    let fixture = Fixture::new();
    let database = fixture.database("env", "78");
    let ssh = fixture.fake_ssh(255);
    let outcome = run(&connection("buildbox"), None, &env(&[("TERMINFO", &database)]), &ssh);
    assert_eq!(outcome, Ok(Outcome::Exited(255)));
}

#[test]
fn a_missing_ssh_is_a_spawn_failure() {
    let fixture = Fixture::new();
    let database = fixture.database("env", "78");
    let missing = fixture.path("no-ssh").display().to_string();
    let outcome = run(
        &connection("buildbox"),
        None,
        &env(&[("TERMINFO", &database)]),
        &missing,
    );
    #[cfg(unix)]
    assert_eq!(
        outcome,
        Err(InstallFailure::SpawnFailed {
            operation: "posix_spawnp".to_owned(),
            reason: "No such file or directory".to_owned()
        })
    );
    // the text is the OS's, or std's own when it finds no such program before asking the OS
    #[cfg(windows)]
    assert!(
        matches!(&outcome, Err(InstallFailure::SpawnFailed { operation, .. }) if operation == "posix_spawnp"),
        "{outcome:?}"
    );
}

#[test]
#[cfg(unix)]
fn ssh_stays_in_the_callers_process_group() {
    let fixture = Fixture::new();
    let database = fixture.database("env", "78");
    let record = fixture.path("ssh.pgid");
    let ssh = fixture.fake_ssh(
        0,
        true,
        &format!("ps -o pgid= -p $$ | tr -d ' ' > '{}'", record.display()),
    );
    let environment = env(&[("TERMINFO", &database), ("PATH", Path::new("/bin:/usr/bin"))]);
    let outcome = run(&connection("buildbox"), None, &environment, &ssh);
    assert_eq!(outcome, Ok(Outcome::Exited(0)));
    // SAFETY: `getpgrp` only reads the calling process's group.
    let group = unsafe { libc::getpgrp() };
    assert_eq!(fixture.read("ssh.pgid").trim(), group.to_string());
}

#[test]
#[cfg(unix)]
fn ssh_inherits_no_descriptor_beyond_its_standard_three() {
    let fixture = Fixture::new();
    let database = fixture.database("env", "78");
    // a descriptor without close-on-exec, as one a caller leaked would be
    let mut stray = [-1; 2];
    // SAFETY: `stray` is a valid two-element array for `pipe` to fill.
    assert_eq!(unsafe { libc::pipe(stray.as_mut_ptr()) }, 0);
    // the probe writes to the pipe's number from a child shell: ssh's own descriptors (dash's script, from 10 up, and
    // a stdout either shell parks while a redirect is in force) may take that number with nothing leaked, but they
    // are close-on-exec, so only a leaked pipe carries the write to the read end. `/proc` would name the descriptor,
    // but on Linux alone.
    let probe = format!("/bin/sh -c 'echo inherited >&{fd}' 2> /dev/null", fd = stray[1]);
    let ssh = fixture.fake_ssh(0, true, &probe);
    let environment = env(&[("TERMINFO", &database), ("PATH", Path::new("/bin:/usr/bin"))]);
    let outcome = run(&connection("buildbox"), None, &environment, &ssh);
    // a sibling test's child may hold the write end too, so the read never waits for its end
    // SAFETY: `fcntl` on the read end `pipe` opened above.
    assert_eq!(unsafe { libc::fcntl(stray[0], libc::F_SETFL, libc::O_NONBLOCK) }, 0);
    let mut leaked = [0_u8; 64];
    // SAFETY: `leaked` is a valid, writable buffer of its length.
    let read = unsafe { libc::read(stray[0], leaked.as_mut_ptr().cast(), leaked.len()) };
    let error = io::Error::last_os_error();
    for fd in stray {
        // SAFETY: closing the two descriptors `pipe` opened above, once each.
        unsafe { libc::close(fd) };
    }
    assert_eq!(outcome, Ok(Outcome::Exited(0)));
    assert_eq!(read, -1, "ssh inherited the pipe");
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
}

/// Runs `body` with `signal` blocked on this thread only, the previous mask restored after.
#[cfg(unix)]
fn with_blocked<T>(signal: i32, body: impl FnOnce() -> T) -> T {
    // SAFETY: a zeroed set is initialized by `sigemptyset` before use.
    let mut blocked: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: a zeroed set is written by `pthread_sigmask` before use.
    let mut previous: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: `blocked` is a valid set.
    unsafe { libc::sigemptyset(&raw mut blocked) };
    // SAFETY: `blocked` is a valid set and `signal` a valid signal.
    unsafe { libc::sigaddset(&raw mut blocked, signal) };
    // SAFETY: both sets are valid; the mask is this thread's.
    let set = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &raw const blocked, &raw mut previous) };
    assert_eq!(set, 0);
    let result = body();
    // SAFETY: `previous` was written by the call above.
    unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &raw const previous, std::ptr::null_mut()) };
    result
}

#[test]
#[cfg(unix)]
fn ssh_starts_with_signals_unblocked_whatever_the_caller_blocks() {
    let fixture = Fixture::new();
    let database = fixture.database("env", "78");
    let ssh = fixture.fake_ssh(0, true, "kill -TERM $$");
    let environment = env(&[("TERMINFO", &database), ("PATH", Path::new("/bin:/usr/bin"))]);
    let outcome = with_blocked(libc::SIGTERM, || run(&connection("buildbox"), None, &environment, &ssh));
    assert_eq!(outcome, Ok(Outcome::Signaled(libc::SIGTERM)));
}

#[test]
#[cfg(unix)]
fn an_ssh_that_exits_before_reading_stdin_still_reports_its_status() {
    // with SIGPIPE at its default, as the real binary runs (a real entry's source stays under a pipe buffer, so only
    // an entry made this large reaches this end to end)
    crate::local::default_sigpipe::run(
        "terminfo::tests::an_ssh_that_exits_before_reading_stdin_still_reports_its_status",
        || {
            let fixture = Fixture::new();
            // a value of control characters, each four bytes of source: larger than a pipe buffer, so the write would
            // block or EPIPE against a reader that has gone
            let value = vec![1_u8; 30_000];
            let mut strings = vec![Slot::Absent; 1];
            strings[0] = Slot::Value(&value);
            let large = Compiled {
                names: ENTRY,
                booleans: &[],
                numbers: &[],
                strings: &strings,
                extended: &[],
                wide: false,
            };
            let database = fixture.database_with("env", "78", &large.bytes());
            assert!(dump(&database).unwrap().len() > 1 << 16);
            let ssh = fixture.fake_ssh(255, false, "");
            let outcome = run(&connection("buildbox"), None, &env(&[("TERMINFO", &database)]), &ssh);
            assert_eq!(outcome, Ok(Outcome::Exited(255)));
        },
    );
}

// -- a real compiled entry

#[test]
#[cfg(unix)]
fn an_entry_tic_compiled_reaches_ssh_as_source() {
    if !Path::new("/usr/bin/tic").exists() {
        return;
    }
    let fixture = Fixture::new();
    let database = fixture.compiled_database("xterm-ghostty|ngterm test entry,\n\tcols#80, lines#24,\n");
    let ssh = fixture.fake_ssh(0, true, "");
    let outcome = run(&connection("buildbox"), None, &env(&[("TERMINFO", &database)]), &ssh);
    assert_eq!(outcome, Ok(Outcome::Exited(0)));
    let dumped = fixture.read("ssh.stdin");
    let (_, body) = dumped.split_once('\n').unwrap();
    assert_eq!(body, "xterm-ghostty|ngterm test entry,\n\tcols#80, lines#24,\n");
}
