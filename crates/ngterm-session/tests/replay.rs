//! A live pane's replay script (`ngterm-core`'s `live::replay`) on a real PTY, under each Unix login shell this host
//! has (zsh, bash, fish), the way a pane's daemon runs it (`<shell> <script flags> <script>`): a command interrupted
//! with SIGINT, as Ctrl+C interrupts it, still ends at the final shell with the integration re-armed (upstream's
//! `interruptedPayloadStillExecsTheFinalShell`, #773). The interactive shell that runs the script drops the rest of a
//! plain `;` list once its foreground job is interrupted, so without each shell's form the pane's process would end
//! and the pane would close. Both forms: a captured argv and a durable creation line, the latter also as a list. A
//! hangup, how the session server ends a program, ends the session instead, the final shell never started, and so
//! does an explicit `exit` in a durable line (bash's interrupt-only `EXIT` trap; fish's form; zsh's `always` block
//! is upstream's own, so only the hangup is checked under zsh).
//!
//! The command and the final "shell" are stand-ins: the command logs that it ran, records its process group and the
//! terminal's foreground group, then sends SIGINT to its group; the final shell records the variables the replay
//! re-armed and the flags it was exec'd with, then ends, which ends the session. Each shell runs without the user's
//! startup files (a throwaway home, a `ZDOTDIR` holding only an empty `.zshenv`, `fish --no-config`) and without the
//! live ngterm's `NGTERM_*`, `AGTERM_*` and `ZMX_*` variables, with a namespace directory of its own (only named in its
//! environment: no server runs). Windows has none of these shells: its live pane runs PowerShell, whose interrupted
//! replay `pwsh.rs` covers.
#![cfg(unix)]

#[expect(dead_code, reason = "the helpers serve the pane, the lead and these tests alike")]
mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use ngterm_core::command_restore::shell_quoted_line;
use ngterm_core::live::replay::{self, FinalShell};
use ngterm_core::live::shell_integration::{LoginShell, Rearm};
use ngterm_session::protocol::{ExitStatus, SessionSpec};
use ngterm_session::server::pane::Pane;
use support::{Client, TestResult, config};

/// A login shell this host has, with where it is.
struct Shell {
    kind: LoginShell,
    path: &'static str,
}

/// The Unix login shells this host has. macOS' `/bin/bash` is bash 3.2, which a live pane never runs (Ghostty
/// injects no integration into it), so only a newer bash counts there.
fn shells() -> Vec<Shell> {
    let bash: &[&'static str] = if cfg!(target_os = "macos") {
        &["/opt/homebrew/bin/bash", "/usr/local/bin/bash"]
    } else {
        &["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"]
    };
    let zsh: &[&'static str] = &["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"];
    let fish: &[&'static str] = &[
        "/usr/bin/fish",
        "/bin/fish",
        "/usr/local/bin/fish",
        "/opt/homebrew/bin/fish",
    ];
    [
        (LoginShell::Zsh, zsh),
        (LoginShell::Bash, bash),
        (LoginShell::Fish, fish),
    ]
    .into_iter()
    .filter_map(|(kind, paths)| {
        let path = paths.iter().copied().find(|path| Path::new(path).exists());
        if path.is_none() {
            eprintln!("SKIPPED: no {} on this host", kind.name());
        }
        path.map(|path| Shell { kind, path })
    })
    .collect()
}

/// What each shell's replay re-arms before its final shell: values naming no real integration, since the final shell
/// is a stand-in that only records them.
fn rearm(kind: LoginShell) -> Vec<Rearm> {
    match kind {
        LoginShell::Zsh => vec![
            Rearm::Export("ZDOTDIR", "/bundle/integration".to_owned()),
            Rearm::Export("GHOSTTY_ZSH_ZDOTDIR", "/user/zsh".to_owned()),
        ],
        LoginShell::Bash => vec![
            Rearm::Export("ENV", "/bundle/bash/ghostty.bash".to_owned()),
            Rearm::Export("GHOSTTY_BASH_INJECT", "1".to_owned()),
            Rearm::Unset("GHOSTTY_BASH_ENV"),
        ],
        LoginShell::Fish => vec![
            Rearm::Export("GHOSTTY_SHELL_INTEGRATION_XDG_DIR", "/integration root".to_owned()),
            Rearm::FishDataDirs("/integration root".to_owned()),
        ],
        LoginShell::Pwsh => unreachable!("PowerShell has no exec: pwsh.rs covers its replay"),
    }
}

/// The `XDG_DATA_DIRS` a fish session runs with, which its re-arm puts the integration root in front of.
const FISH_DATA_DIRS: &str = "/user/share";

/// The lines the final shell records for `rearm` and `kind`'s interactive flags: `KEY=value` for an export,
/// `KEY unset` for an unset, then `args=<flags>`.
fn expected_record(kind: LoginShell, rearm: &[Rearm]) -> String {
    let mut lines: Vec<String> = rearm
        .iter()
        .map(|step| match step {
            Rearm::Export(key, value) => format!("{key}={value}\n"),
            Rearm::Unset(key) => format!("{key} unset\n"),
            // recomputed from the session's own value, which the command left alone
            Rearm::FishDataDirs(root) => format!("{}={root}:{FISH_DATA_DIRS}\n", step.key()),
        })
        .collect();
    lines.push(format!("args={}\n", kind.interactive_flags().join(" ")));
    lines.concat()
}

/// The final shell's stand-in: it appends what [`expected_record`] describes to `$RESULT_FILE`.
fn final_shell_script(rearm: &[Rearm]) -> String {
    let checks: String = rearm
        .iter()
        .map(|step| {
            let key = step.key();
            format!(
                "if [ -n \"${{{key}+set}}\" ]; then printf '%s\\n' \"{key}=${key}\"; else printf '%s\\n' '{key} unset'; fi\n"
            )
        })
        .collect::<Vec<_>>()
        .concat();
    format!("#!/bin/sh\n{{\n{checks}printf 'args=%s\\n' \"$*\"\n}} >> \"$RESULT_FILE\"\n")
}

/// The interrupted command: it logs that it ran, records its process group and the terminal's foreground group, and
/// sends SIGINT to its group, as Ctrl+C does to the pane's foreground job.
const PAYLOAD: &str = "#!/bin/sh\n\
printf 'ran\\n' >> \"$PAYLOAD_LOG\"\n\
ps -o pgid= -o tpgid= -p $$ > \"$GROUP_FILE\"\n\
kill -INT 0\n\
printf 'survived\\n' >> \"$PAYLOAD_LOG\"\n";

/// The command a hangup ends: it logs that it ran, records its process group and the terminal's foreground group, and
/// waits.
const LONG_PAYLOAD: &str = "#!/bin/sh\n\
printf 'ran\\n' >> \"$PAYLOAD_LOG\"\n\
ps -o pgid= -o tpgid= -p $$ > \"$GROUP_FILE\"\n\
sleep 60\n\
printf 'survived\\n' >> \"$PAYLOAD_LOG\"\n";

/// The replay's two forms, and the durable line as a list too, whose interruption bash drops the whole string for.
#[derive(Debug, Clone, Copy)]
enum Form {
    Argv,
    Line,
    List,
}

/// The session a live pane's daemon runs for `script` under `shell`, in `dir`'s throwaway home.
fn session(shell: &Shell, script: String, dir: &Path) -> TestResult<SessionSpec> {
    let home = dir.join("home");
    let zdotdir = dir.join("zdotdir");
    fs::create_dir_all(&home)?;
    fs::create_dir_all(&zdotdir)?;
    // an interactive zsh finding no startup file at all runs `zsh-newuser-install`, which waits for a key
    fs::write(zdotdir.join(".zshenv"), "")?;
    let mut environment: BTreeMap<String, String> = support::environment().into_iter().collect();
    for key in [
        "ENV",
        "BASH_ENV",
        "GHOSTTY_BASH_INJECT",
        "GHOSTTY_BASH_ENV",
        "GHOSTTY_ZSH_ZDOTDIR",
        "GHOSTTY_SHELL_INTEGRATION_XDG_DIR",
        "XDG_DATA_DIRS",
    ] {
        environment.remove(key);
    }
    for (key, value) in [
        ("HOME", home.clone()),
        ("ZDOTDIR", zdotdir),
        ("XDG_CONFIG_HOME", home.join(".config")),
        ("XDG_DATA_HOME", home.join(".local/share")),
        ("XDG_CACHE_HOME", home.join(".cache")),
        ("PAYLOAD_LOG", dir.join("payload-log")),
        ("GROUP_FILE", dir.join("group")),
        ("RESULT_FILE", dir.join("result")),
    ] {
        environment.insert(key.to_owned(), value.display().to_string());
    }
    environment.insert("SHELL".to_owned(), shell.path.to_owned());
    environment.insert("TERM".to_owned(), "xterm-256color".to_owned());
    if shell.kind.is_fish() {
        environment.insert("XDG_DATA_DIRS".to_owned(), FISH_DATA_DIRS.to_owned());
    }
    let mut argv = vec![shell.path.to_owned()];
    if shell.kind.is_fish() {
        // no /etc or vendor configuration of the host's fish
        argv.push("--no-config".to_owned());
    }
    argv.extend(shell.kind.script_flags().iter().map(|flag| (*flag).to_owned()));
    argv.push(script);
    Ok(SessionSpec {
        argv,
        env: environment.into_iter().collect(),
        cwd: Some(dir.display().to_string()),
        tracked_env: Vec::new(),
    })
}

/// The replay script of `form` running the payload at `payload` under `shell`.
fn script(shell: &Shell, form: Form, payload: &Path, final_shell: &FinalShell<'_>) -> String {
    let payload = payload.display().to_string();
    let quote = |word: &str| {
        if shell.kind.is_fish() {
            replay::fish_quoted(word)
        } else {
            shell_quoted_line(&[word])
        }
    };
    match form {
        Form::Argv => replay::render_argv(&[payload], final_shell),
        Form::Line => replay::render_command_line(&quote(&payload), final_shell),
        Form::List => {
            let line = format!("{} && printf 'list went on\\n' >> \"$PAYLOAD_LOG\"", quote(&payload));
            replay::render_command_line(&line, final_shell)
        }
    }
}

fn executable(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, contents)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

/// The final shell's stand-in for `shell`, written into `dir`, with what the replay re-arms before it; its path.
fn final_shell_probe(shell: &Shell, dir: &Path) -> TestResult<(String, Vec<Rearm>)> {
    let rearm = rearm(shell.kind);
    let probe = dir.join("final shell");
    executable(&probe, &final_shell_script(&rearm))?;
    Ok((probe.display().to_string(), rearm))
}

/// Spawns the pane a live session's daemon runs for the replay `script` under `shell`, named after `what`.
fn spawn_replay(shell: &Shell, script: String, dir: &Path, what: &str) -> TestResult<(Pane, Client)> {
    let name = format!("replay-{}-{what}", shell.kind.name());
    let (pane, creator) = Pane::spawn(config(&name, dir, session(shell, script, dir)?))?;
    Ok((pane, Client::new(creator)))
}

/// Runs one interrupted replay of `form` under `shell` and checks it: the payload ran once, in a process group of its
/// own that held the terminal's foreground, and the final shell was exec'd once with the integration re-armed.
fn interrupted_replay(shell: &Shell, form: Form) -> TestResult {
    let dir = tempfile::tempdir()?;
    let payload = dir.path().join("interrupted payload");
    executable(&payload, PAYLOAD)?;
    let (probe_path, rearm) = final_shell_probe(shell, dir.path())?;
    let final_shell = FinalShell {
        path: &probe_path,
        kind: shell.kind,
        rearm: &rearm,
    };
    let script = script(shell, form, &payload, &final_shell);
    let (pane, mut client) = spawn_replay(shell, script, dir.path(), &format!("{form:?}"))?;
    let status = client.finish();
    let what = format!("{} {form:?}", shell.kind.name());
    let log = fs::read_to_string(dir.path().join("payload-log")).unwrap_or_default();
    assert_eq!(
        log,
        "ran\n",
        "{what}: the payload runs once and its list ends with it: {}",
        client.text()
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("result")).unwrap_or_default(),
        expected_record(shell.kind, &rearm),
        "{what}: the final shell is exec'd once, re-armed: {}",
        client.text()
    );
    assert!(
        matches!(status, ExitStatus::Exited { code: 0, .. }),
        "{what}: {status:?}"
    );
    // the payload was the pane's foreground job, in a group of its own, not the shell's
    let group = fs::read_to_string(dir.path().join("group"))?;
    let ids: Vec<u32> = group.split_whitespace().map(str::parse).collect::<Result<_, _>>()?;
    assert_eq!(ids.len(), 2, "{what}: {group:?}");
    assert_eq!(
        ids[0], ids[1],
        "{what}: the payload's group holds the foreground: {group:?}"
    );
    assert_ne!(
        ids[0],
        pane.pid(),
        "{what}: the payload has a group of its own: {group:?}"
    );
    Ok(())
}

#[test]
fn an_interrupted_command_still_ends_at_the_rearmed_login_shell() -> TestResult {
    for shell in shells() {
        for form in [Form::Argv, Form::Line, Form::List] {
            interrupted_replay(&shell, form)?;
        }
    }
    Ok(())
}

/// Hangs up a replay of `form` under `shell` while its command runs in the foreground, as `live.prune` and an unforced
/// `live.kill` do: the session ends within the kill's grace, and the final shell is never started.
fn hung_up_replay(shell: &Shell, form: Form) -> TestResult {
    let dir = tempfile::tempdir()?;
    let payload = dir.path().join("long payload");
    executable(&payload, LONG_PAYLOAD)?;
    let (probe_path, rearm) = final_shell_probe(shell, dir.path())?;
    let final_shell = FinalShell {
        path: &probe_path,
        kind: shell.kind,
        rearm: &rearm,
    };
    let script = script(shell, form, &payload, &final_shell);
    let (pane, mut client) = spawn_replay(shell, script, dir.path(), &format!("hangup-{form:?}"))?;
    let what = format!("{} {form:?} hung up", shell.kind.name());
    // the payload holds the terminal's foreground before the hangup
    support::wait_for(
        &format!("{what}: the payload in the foreground"),
        || fs::read_to_string(dir.path().join("group")).unwrap_or_default(),
        |group| {
            let ids: Vec<&str> = group.split_whitespace().collect();
            ids.len() == 2 && ids[0] == ids[1]
        },
    );
    assert!(
        pane.kill(false)?,
        "{what}: the session ends on its hangup: {}",
        client.text()
    );
    client.finish();
    assert_eq!(
        fs::read_to_string(dir.path().join("result")).unwrap_or_default(),
        "",
        "{what}: no final shell is started: {}",
        client.text()
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("payload-log")).unwrap_or_default(),
        "ran\n",
        "{what}: {}",
        client.text()
    );
    Ok(())
}

#[test]
fn a_hangup_ends_the_session_without_starting_the_login_shell() -> TestResult {
    for shell in shells() {
        for form in [Form::Argv, Form::Line, Form::List] {
            hung_up_replay(&shell, form)?;
        }
    }
    Ok(())
}

/// The durable lines whose `exit` ends the session: after a command that succeeded, and after one that returned 130
/// itself, the status an interrupted pipeline reads in bash's `PIPESTATUS` too.
const EXIT_LINES: [(&str, &str); 2] = [
    ("after-success", "printf 'ran\\n' >> \"$PAYLOAD_LOG\"; exit 3"),
    (
        "after-130",
        "printf 'ran\\n' >> \"$PAYLOAD_LOG\"; /bin/sh -c 'exit 130'; exit 3",
    ),
];

#[test]
fn an_exit_in_a_durable_line_ends_the_session_with_its_status() -> TestResult {
    for shell in shells().into_iter().filter(|shell| shell.kind != LoginShell::Zsh) {
        for (case, line) in EXIT_LINES {
            exited_replay(&shell, case, line)?;
        }
    }
    Ok(())
}

/// Runs the durable `line`, which ends in `exit 3`, under `shell` and checks that the session ended with that status,
/// the final shell never started.
fn exited_replay(shell: &Shell, case: &str, line: &str) -> TestResult {
    let dir = tempfile::tempdir()?;
    let (probe_path, rearm) = final_shell_probe(shell, dir.path())?;
    let final_shell = FinalShell {
        path: &probe_path,
        kind: shell.kind,
        rearm: &rearm,
    };
    let script = replay::render_command_line(line, &final_shell);
    let (_pane, mut client) = spawn_replay(shell, script, dir.path(), &format!("exit-{case}"))?;
    let status = client.finish();
    let what = format!("{} exit {case}", shell.kind.name());
    assert!(
        matches!(status, ExitStatus::Exited { code: 3, .. }),
        "{what}: {status:?}: {}",
        client.text()
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("payload-log")).unwrap_or_default(),
        "ran\n",
        "{what}: {}",
        client.text()
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("result")).unwrap_or_default(),
        "",
        "{what}: no final shell is started: {}",
        client.text()
    );
    Ok(())
}
