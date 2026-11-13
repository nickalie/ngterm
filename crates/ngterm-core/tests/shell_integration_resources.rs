//! Ghostty's Bash and Zsh integrations as this build stages them (`scripts/build-ghostty-vt.sh` copies the pinned
//! revision's `src/shell-integration` into its prefix's `share/ghostty`, `scripts/stage.sh` from there into the app),
//! started the way a pane's shell is (`ngterm_core::live::shell_integration::inject` and the login shell's interactive
//! flags), report a working directory whose name carries escape sequences without letting one through (Ghostty's
//! #14859): each report is one `ESC ] 7 ; kitty-shell-cwd://<host><path> BEL` with the control bytes left out of the
//! host and the path, so a directory named `x\a\e]52;c;SGVsbG8=\a` (backslashes as written) stays literal text, one
//! with a raw ESC and BEL in its name loses them, and neither writes the clipboard with an OSC 52; spaces, Unicode
//! and backslashes are kept.
//!
//! Unix only (no Windows pane runs these shells). Each shell runs in a throwaway home with the test's `PATH` and no
//! other inherited variable (no `GHOSTTY_RESOURCES_DIR` of another terminal), without the system's startup files
//! (a distribution's `/etc/profile` or `/etc/bash.bashrc` may title the terminal with the raw directory and host
//! itself, which is not the integration's to fix): Bash through the integration's own `--noprofile` (the
//! `GHOSTTY_BASH_INJECT` value Ghostty writes for a shell started with it), zsh through the home's `.zshenv`, which
//! the integration sources and which turns `GLOBAL_RCS` off. It reads its command lines from a pipe,
//! in a session of its own with no controlling terminal, so the zsh integration, which prefers the terminal to its
//! standard output, writes to the output this test reads and never to the terminal the test runs in. A shell that is
//! not installed (zsh), or only an unsupported Bash (macOS's `/bin/bash` 3.2), skips with a note.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use ngterm_core::live::shell_integration::{self, Inherited, LoginShell};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// How long a shell may take to read its lines and exit.
const PATIENCE: Duration = Duration::from_secs(60);

/// The directories the shell changes into, below the work directory: what the name is on disk, and what a report
/// carries in its place.
const DIRECTORIES: [(&str, &str); 3] = [
    // Ghostty's case: backslash escapes as written, which a shell's `print` (zsh's without `-r`) would expand
    (r"x\a\e]52;c;SGVsbG8=\a", r"x\a\e]52;c;SGVsbG8=\a"),
    // the raw bytes: a BEL to end the report, then an OSC 52 of its own
    ("y\u{7}\u{1b}]52;c;SGVsbG8=\u{7}", "y]52;c;SGVsbG8="),
    (
        "my dir \u{fc}\u{1f600} back\\slash",
        "my dir \u{fc}\u{1f600} back\\slash",
    ),
];

/// A hostname with an OSC 52 of its own, set in the shell's hostname variable before the last change of directory,
/// and what the report carries in its place.
const HOSTILE_HOST: (&str, &str) = ("h\u{7}\u{1b}]52;c;SGVsbG8=\u{7}ost", "h]52;c;SGVsbG8=ost");

/// The staged Ghostty resources of this build: `share/ghostty` under the prefix `scripts/build-ghostty-vt.sh
/// --print-prefix` names, which that script prepares before any build of the workspace.
fn staged_resources() -> TestResult<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("bash")
        .arg(root.join("scripts/build-ghostty-vt.sh"))
        .arg("--print-prefix")
        .current_dir(&root)
        .output()?;
    assert!(output.status.success(), "{output:?}");
    let prefix = String::from_utf8(output.stdout)?;
    let resources = PathBuf::from(prefix.trim()).join("share").join("ghostty");
    for shell in [LoginShell::Bash, LoginShell::Zsh] {
        assert!(
            shell.is_bundled(&resources),
            "no staged {} integration in {}: run scripts/build-ghostty-vt.sh",
            shell.name(),
            resources.display()
        );
    }
    Ok(resources)
}

/// The first `name` on `PATH` that `usable` accepts.
fn find_shell(name: &str, usable: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file() && usable(path))
}

/// A Bash the integration supports, 4 or newer: not macOS's `/bin/bash` 3.2.
fn bash() -> Option<PathBuf> {
    find_shell("bash", |path| {
        Command::new(path)
            .args(["-c", "echo ${BASH_VERSINFO[0]}"])
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|major| major.trim().parse::<u32>().ok())
            .is_some_and(|major| major >= 4)
    })
}

fn zsh() -> Option<PathBuf> {
    find_shell("zsh", |_| true)
}

/// The OSC payloads in `output` (between `ESC ]` and a BEL or an ST), in order.
fn osc_payloads(output: &[u8]) -> Vec<Vec<u8>> {
    let mut payloads = Vec::new();
    let mut rest = output;
    while let Some(start) = rest.windows(2).position(|window| window == b"\x1b]") {
        rest = &rest[start + 2..];
        let end = rest
            .iter()
            .position(|byte| *byte == 0x07 || *byte == 0x1b)
            .unwrap_or(rest.len());
        payloads.push(rest[..end].to_vec());
        rest = &rest[end..];
    }
    payloads
}

/// Runs `shell` (`kind`) the way a pane starts it, in `work` with `home` as its home, reading `script` from its
/// standard input; its output once it exits.
fn run_integrated(
    kind: LoginShell,
    shell: &Path,
    resources: &Path,
    dirs: (&Path, &Path),
    script: &str,
) -> TestResult<Output> {
    run_integrated_with(kind, shell, resources, dirs, script, |_| {})
}

/// [`run_integrated`] with the shell's variables changed by `adjust` once the integration set them (`PATH` among
/// them, which is otherwise the test's).
fn run_integrated_with(
    kind: LoginShell,
    shell: &Path,
    resources: &Path,
    dirs: (&Path, &Path),
    script: &str,
    adjust: impl FnOnce(&mut BTreeMap<String, String>),
) -> TestResult<Output> {
    let (home, work) = dirs;
    let mut environment = BTreeMap::new();
    let inherited = Inherited {
        home: Some(home.display().to_string()),
        ..Inherited::default()
    };
    shell_integration::inject(kind, resources, &inherited, &mut environment);
    if kind == LoginShell::Bash {
        environment.insert("GHOSTTY_BASH_INJECT".to_owned(), "1 --noprofile".to_owned());
    }
    adjust(&mut environment);
    let mut command = Command::new(shell);
    command
        .args(kind.interactive_flags())
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("TERM", "xterm-256color")
        .env("GHOSTTY_SHELL_FEATURES", "title")
        .envs(&environment)
        .current_dir(work)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (index, (name, _)) in DIRECTORIES.iter().enumerate() {
        command.env(format!("NGTERM_TEST_DIR{index}"), work.join(name));
    }
    command.env("NGTERM_TEST_HOST", HOSTILE_HOST.0);
    // SAFETY: the hook, `detach`, makes only an async-signal-safe system call between `fork` and `exec`.
    unsafe { command.pre_exec(detach) };
    let mut child = command.spawn()?;
    child
        .stdin
        .take()
        .ok_or("no input to the shell")?
        .write_all(script.as_bytes())?;
    let (sender, receiver) = mpsc::channel();
    let pid = libc::pid_t::try_from(child.id())?;
    std::thread::spawn(move || sender.send(child.wait_with_output()));
    let Ok(output) = receiver.recv_timeout(PATIENCE) else {
        // SAFETY: a plain `kill(2)` of the child the test started, which the waiting thread has not reaped yet.
        unsafe { libc::kill(pid, libc::SIGKILL) };
        return Err(format!("{} did not exit within {PATIENCE:?}", kind.name()).into());
    };
    Ok(output?)
}

/// The child's hook before `exec`: a session of its own, so it has no controlling terminal.
fn detach() -> std::io::Result<()> {
    // SAFETY: `setsid` takes no argument and is async-signal-safe.
    if unsafe { libc::setsid() } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// The integration of `kind` run by `shell` reports each directory the script changes into once per report, with
/// the control bytes left out of host and path, and never lets an OSC 52 through.
fn reports_safely(kind: LoginShell, shell: &Path, host_variable: &str) -> TestResult {
    let resources = staged_resources()?;
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let home = root.join("home");
    let work = root.join("work");
    fs::create_dir_all(&home)?;
    // no startup file of the system's after this one, and no first-run wizard
    fs::write(home.join(".zshenv"), "unsetopt GLOBAL_RCS\n")?;
    fs::write(home.join(".zshrc"), "")?;
    for (name, _) in DIRECTORIES {
        fs::create_dir_all(work.join(name))?;
    }
    let changes = (0..DIRECTORIES.len())
        .map(|index| format!("cd -- \"$NGTERM_TEST_DIR{index}\"\n"))
        .collect::<Vec<_>>()
        .concat();
    // the host changes with the directory, so the one report after them carries both
    let script = format!("{changes}{host_variable}=\"$NGTERM_TEST_HOST\"; cd ..\nexit\n");
    let output = run_integrated(kind, shell, &resources, (&home, &work), &script)?;
    let name = kind.name();
    assert!(output.status.success(), "{name}: {output:?}");
    for stream in [&output.stdout, &output.stderr] {
        assert!(
            !stream.windows(4).any(|window| window == b"\x1b]52"),
            "{name}: an OSC 52 got through: {output:?}"
        );
    }

    // every report is one OSC 7, ended by its BEL, with no control byte inside
    let stdout = &output.stdout;
    let reports = osc_payloads(stdout)
        .into_iter()
        .filter_map(|payload| payload.strip_prefix(b"7;").map(<[u8]>::to_vec))
        .map(String::from_utf8)
        .collect::<Result<Vec<_>, _>>()?;
    let report_count = stdout.windows(4).filter(|window| window == b"\x1b]7;").count();
    assert_eq!(reports.len(), report_count, "{name}: {output:?}");
    let ended = stdout
        .windows(4)
        .enumerate()
        .filter(|(_, window)| *window == b"\x1b]7;")
        .all(|(at, _)| {
            let rest = &stdout[at + 4..];
            rest.iter().position(|byte| *byte == 0x07) == rest.iter().position(u8::is_ascii_control)
        });
    assert!(ended, "{name}: a report not ended by its own BEL: {output:?}");

    // the directories in order, each reported (zsh reports on a change and at each prompt, bash once per change)
    let mut seen: Vec<(String, String)> = Vec::new();
    for report in &reports {
        let rest = report
            .strip_prefix("kitty-shell-cwd://")
            .ok_or_else(|| format!("{name}: {report:?}"))?;
        let (host, path) = rest.split_at(rest.find('/').ok_or_else(|| format!("{name}: {report:?}"))?);
        let entry = (host.to_owned(), path.to_owned());
        if seen.last() != Some(&entry) {
            seen.push(entry);
        }
    }
    let local = seen.first().map(|(host, _)| host.clone()).unwrap_or_default();
    assert!(!local.is_empty(), "{name}: no host in {seen:?}");
    let mut expected = vec![(local.clone(), work.display().to_string())];
    for (_, reported) in DIRECTORIES {
        expected.push((local.clone(), work.join(reported).display().to_string()));
    }
    expected.push((HOSTILE_HOST.1.to_owned(), work.display().to_string()));
    assert_eq!(seen, expected, "{name}: {output:?}");
    Ok(())
}

#[test]
fn bash_reports_a_hostile_directory_or_host_without_its_escape_sequences() -> TestResult {
    let Some(shell) = bash() else {
        eprintln!("SKIPPED: no Bash 4 or newer on PATH");
        return Ok(());
    };
    reports_safely(LoginShell::Bash, &shell, "HOSTNAME")
}

#[test]
fn zsh_reports_a_hostile_directory_or_host_without_its_escape_sequences() -> TestResult {
    let Some(shell) = zsh() else {
        eprintln!("SKIPPED: no zsh on PATH");
        return Ok(());
    };
    reports_safely(LoginShell::Zsh, &shell, "HOST")
}

/// Git Bash on Windows starts ngterm's shim (`resources/shell-integration/git-bash/bash-env`, what `ENV` names there),
/// which loads Ghostty's integration and replaces its directory report, bash's MSYS path, with the Windows form
/// `cygpath -m` gives (faked here: `C:/msys` in front of the path), and none for a directory cygpath cannot convert
/// or whose name holds control bytes (the fake hands them back, so only the shim's own checks keep an OSC 52 out);
/// Ghostty's prompt marks still carry each command's status, and the loader's variable reaches no child process.
#[test]
fn git_bash_shim_reports_cygpaths_windows_form_instead_of_the_msys_path() -> TestResult {
    let Some(shell) = bash() else {
        eprintln!("SKIPPED: no Bash 4 or newer on PATH");
        return Ok(());
    };
    let resources = staged_resources()?;
    let shim = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("resources")
        .join(shell_integration::GIT_BASH_SHIM)
        .canonicalize()?;
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let (home, work, bin) = (root.join("home"), root.join("work"), root.join("bin"));
    fs::create_dir_all(work.join("a b").join("unconvertible"))?;
    // the raw bytes: a BEL to end the report, then an OSC 52 of its own
    fs::create_dir_all(work.join(DIRECTORIES[1].0))?;
    fs::create_dir_all(&home)?;
    fs::create_dir_all(&bin)?;
    let cygpath = bin.join("cygpath");
    fs::write(
        &cygpath,
        "#!/bin/sh\n[ \"$1 $2\" = '-m --' ] || exit 2\ncase $3 in */unconvertible) exit 1 ;; esac\nprintf 'C:/msys%s\\n' \"$3\"\n",
    )?;
    fs::set_permissions(&cygpath, fs::Permissions::from_mode(0o755))?;
    let path = std::env::join_paths(
        std::iter::once(bin.clone()).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())),
    )?;
    let script = format!(
        "printenv {} || echo loader-unset\ncd -- 'a b'\ncd unconvertible\nfalse\ncd ..\ncd -- \"$NGTERM_TEST_DIR1\"\nexit\n",
        shell_integration::GIT_BASH_GHOSTTY_VARIABLE
    );
    let output = run_integrated_with(
        LoginShell::Bash,
        &shell,
        &resources,
        (&home, &work),
        &script,
        |environment| {
            let loader = environment.insert("ENV".to_owned(), shim.display().to_string());
            environment.insert(
                shell_integration::GIT_BASH_GHOSTTY_VARIABLE.to_owned(),
                loader.unwrap_or_default(),
            );
            environment.insert("PATH".to_owned(), path.to_string_lossy().into_owned());
        },
    )?;
    assert!(output.status.success(), "{output:?}");
    assert!(
        !output.stdout.windows(4).any(|window| window == b"\x1b]52"),
        "an OSC 52 got through: {output:?}"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("loader-unset\n"),
        "the shim leaves {} to child processes: {output:?}",
        shell_integration::GIT_BASH_GHOSTTY_VARIABLE
    );
    let payloads = osc_payloads(&output.stdout)
        .into_iter()
        .map(String::from_utf8)
        .collect::<Result<Vec<_>, _>>()?;
    let reports = payloads
        .iter()
        .filter_map(|payload| payload.strip_prefix("7;"))
        .collect::<Vec<_>>();
    let windows = |path: &Path| format!("kitty-shell-cwd://localhost/C:/msys{}", path.display());
    assert_eq!(
        reports,
        [windows(&work), windows(&work.join("a b")), windows(&work.join("a b"))],
        "{output:?}"
    );
    assert!(
        payloads.iter().any(|payload| payload.starts_with("133;D;1;")),
        "the failed command's status reaches the prompt mark: {output:?}"
    );
    Ok(())
}

#[test]
fn osc_payloads_are_split_at_their_terminators() {
    assert_eq!(
        osc_payloads(b"a\x1b]7;x\x07b\x1b]2;t\x1b\\c\x1b]133;A"),
        [b"7;x".to_vec(), b"2;t".to_vec(), b"133;A".to_vec()]
    );
}
