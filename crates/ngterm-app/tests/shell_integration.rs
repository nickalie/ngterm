//! The bash and zsh shell integration the agent integrations install (`resources/agent-status/shell/integration.sh`),
//! sourced for real into bash with a recording stand-in for the agent-status wrapper (`NGTERM_AGENT_BIN`).
//!
//! Ported from upstream's `agtermCore/Tests/agtermCoreTests/AgentStatusIntegrationTests.swift` (v0.39.1), whole, in
//! this build's names. The integration is POSIX shell, so this test is Unix-only; fish's form did not change.
//!
//! The `codex` wrapper is this build's own (`0005-codex-own-process.patch`), run in every form it ships in, for real
//! under each shell found on `PATH` (a missing one prints `SKIPPED:`), with a recording stand-in for `codex`.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ngterm_host::fakes::write_executable;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn integration() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/agent-status/shell/integration.sh")
}

/// Sources the integration into a bash whose `PROMPT_COMMAND` already holds a user entry, leaves the user's last
/// command with `status`, then evaluates `PROMPT_COMMAND` as bash does before a prompt. Returns what the user entry saw
/// and the stand-in's recorded calls.
fn run_prompt(agent_ran: bool, status: i32) -> TestResult<(String, Vec<String>)> {
    let dir = tempfile::tempdir()?;
    let calls = dir.path().join("calls");
    let stub = dir.path().join("agent-status");
    write_executable(
        &stub,
        &format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", calls.display()),
    )?;
    let integration = integration();
    let script = format!(
        "PROMPT_COMMAND='echo \"status=$? active=${{_ags_active-unset}}\"'\n\
         source '{}'\n\
         {}\n\
         (exit {status})\n\
         eval \"$PROMPT_COMMAND\"\n",
        integration.display(),
        if agent_ran { "_ags_active=1" } else { ":" },
    );
    let out = Command::new("bash")
        .args(["--noprofile", "--norc", "-c", &script])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("NGTERM_SESSION_ID", "sid")
        .env("NGTERM_AGENT_BIN", &stub)
        .output()?;
    let seen = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let recorded = fs::read_to_string(&calls)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    Ok((seen, recorded))
}

#[test]
fn precmd_keeps_the_last_command_status_for_later_prompt_command_entries() -> TestResult {
    for agent_ran in [false, true] {
        for status in [0, 3] {
            let (seen, calls) = run_prompt(agent_ran, status)?;
            assert_eq!(
                seen,
                format!("status={status} active=unset"),
                "agent_ran={agent_ran} status={status}"
            );
            let expected: &[&str] = if agent_ran { &["idle"] } else { &[] };
            assert_eq!(calls, expected, "agent_ran={agent_ran} status={status}");
        }
    }
    Ok(())
}

/// A shell the `codex` wrapper ships in.
#[derive(Clone, Copy, Debug)]
enum Shell {
    Bash,
    Zsh,
    Fish,
    Pwsh,
}

impl Shell {
    const ALL: [Self; 4] = [Self::Bash, Self::Zsh, Self::Fish, Self::Pwsh];

    fn program(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
            Self::Pwsh => "pwsh",
        }
    }

    /// The arguments that run `script` with no user configuration.
    fn args(self, script: &str) -> Vec<String> {
        let flags: &[&str] = match self {
            Self::Bash => &["--noprofile", "--norc", "-c"],
            Self::Zsh => &["-f", "-c"],
            Self::Fish => &["--no-config", "-c"],
            Self::Pwsh => &["-NoProfile", "-NonInteractive", "-Command"],
        };
        flags
            .iter()
            .map(|flag| (*flag).to_owned())
            .chain([script.to_owned()])
            .collect()
    }

    /// The line that sources the wrapper's file.
    fn source(self) -> String {
        let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
        let file = match self {
            Self::Bash | Self::Zsh => "agent-status/shell/integration.sh",
            Self::Fish => "agent-status/shell/integration.fish",
            Self::Pwsh => "shell-integration/pwsh/ngterm.ps1",
        };
        let path = resources.join(file);
        match self {
            Self::Pwsh => format!(". '{}'", path.display()),
            _ => format!("source '{}'", path.display()),
        }
    }

    /// A user's own `codex` function, which prints `user`.
    fn user_function(self) -> &'static str {
        match self {
            Self::Bash | Self::Zsh => "codex() { echo user; }",
            Self::Fish => "function codex; echo user; end",
            Self::Pwsh => "function global:codex { 'user' }",
        }
    }

    /// The expression for the last program's exit status.
    fn status(self) -> &'static str {
        match self {
            Self::Bash | Self::Zsh => "$?",
            Self::Fish => "$status",
            Self::Pwsh => "$LASTEXITCODE",
        }
    }

    fn found(self) -> bool {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let found = std::env::split_paths(&path).any(|dir| dir.join(self.program()).is_file());
        if !found {
            eprintln!("SKIPPED: no {} on PATH", self.program());
        }
        found
    }
}

/// The stand-in's `--help`, with and without the flag, by `STUB_HELP`; every other call records its arguments, echoes
/// its input under `STUB_READ_STDIN`, and exits with `STUB_EXIT`.
const CODEX_STUB: &str = r#"#!/bin/sh
if [ "$*" = "--help" ]; then
  echo "${STUB_HELP:-      --no-daemon   Run without the shared background server}"
  exit 0
fi
printf '%s\n' "$*" >> "$STUB_CALLS"
[ -z "$STUB_READ_STDIN" ] || cat
exit "${STUB_EXIT:-0}"
"#;

/// What one run of `script` under `shell` left behind.
struct Run {
    stdout: String,
    calls: Vec<String>,
}

/// Runs `script` under `shell` with the stand-in first on `PATH`, in an ngterm pane unless `env` says otherwise.
fn run_wrapper(shell: Shell, script: &str, env: &[(&str, &str)]) -> TestResult<Run> {
    let dir = tempfile::tempdir()?;
    let calls = dir.path().join("calls");
    write_executable(&dir.path().join("codex"), CODEX_STUB)?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(std::iter::once(dir.path().to_path_buf()).chain(std::env::split_paths(&path)))?;
    let mut command = Command::new(shell.program());
    command
        .args(shell.args(script))
        .env_clear()
        .env("PATH", path)
        .env("HOME", dir.path())
        .env("STUB_CALLS", &calls)
        .env("NGTERM_SESSION_ID", "sid");
    for (name, value) in env {
        match *value {
            "" => command.env_remove(name),
            value => command.env(name, value),
        };
    }
    let out = command.output()?;
    assert!(
        out.status.success(),
        "{shell:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(Run {
        stdout: String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        calls: fs::read_to_string(&calls)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect(),
    })
}

/// The one call `codex <line>` makes under `shell`, after sourcing the wrapper.
fn wrapped(shell: Shell, line: &str, env: &[(&str, &str)]) -> TestResult<String> {
    let script = format!("{}\ncodex {line}\n", shell.source());
    let calls = run_wrapper(shell, &script, env)?.calls;
    assert_eq!(calls.len(), 1, "{shell:?} `codex {line}`: {calls:?}");
    Ok(calls[0].clone())
}

#[test]
fn codex_tuis_get_their_own_process_and_everything_else_passes_through() -> TestResult {
    let cases = [
        // the TUI's own forms
        ("", "--no-daemon"),
        ("'fix the bug'", "--no-daemon fix the bug"),
        ("-m o3", "--no-daemon -m o3"),
        ("resume --last", "--no-daemon resume --last"),
        ("fork abc", "--no-daemon fork abc"),
        // subcommands, the daemon's own among them
        ("exec hi", "exec hi"),
        ("agents", "agents"),
        ("app-server daemon stop", "app-server daemon stop"),
        ("migrate-rollouts", "migrate-rollouts"),
        // a global option can come first, and Codex refuses the flag on the daemon's own
        ("-c x=y agents", "-c x=y agents"),
        ("-c x=y queue abc hi", "-c x=y queue abc hi"),
        ("-c x=y remote-control pair", "-c x=y remote-control pair"),
        ("-c x=y app-server daemon stop", "-c x=y app-server daemon stop"),
        ("-c x=y resume --last", "--no-daemon -c x=y resume --last"),
        // already decided on the command line: the flag twice is an error, and --remote refuses it
        ("--no-daemon -m o3", "--no-daemon -m o3"),
        ("-m o3 --no-daemon", "-m o3 --no-daemon"),
        ("--remote unix://sock", "--remote unix://sock"),
        ("--remote=unix://sock", "--remote=unix://sock"),
    ];
    for shell in Shell::ALL.into_iter().filter(|shell| shell.found()) {
        for (line, expected) in cases {
            assert_eq!(wrapped(shell, line, &[])?, expected, "{shell:?} `codex {line}`");
        }
        // past `--` it is the prompt's text; PowerShell drops a bare `--` before a function sees it, so it needs quotes
        let separator = match shell {
            Shell::Pwsh => "-m o3 '--' --remote",
            _ => "-m o3 -- --remote",
        };
        assert_eq!(
            wrapped(shell, separator, &[])?,
            "--no-daemon -m o3 -- --remote",
            "{shell:?}"
        );
    }
    Ok(())
}

#[test]
fn codex_stays_on_the_daemon_when_asked_or_when_it_has_no_such_flag() -> TestResult {
    for shell in Shell::ALL.into_iter().filter(|shell| shell.found()) {
        let keep = [("NGTERM_CODEX_KEEP_DAEMON", "1")];
        assert_eq!(wrapped(shell, "-m o3", &keep)?, "-m o3", "{shell:?}");
        let older = [("STUB_HELP", "      --oss   Use a local provider")];
        assert_eq!(wrapped(shell, "-m o3", &older)?, "-m o3", "{shell:?}");
    }
    Ok(())
}

#[test]
fn outside_ngterm_codex_is_not_wrapped() -> TestResult {
    for shell in Shell::ALL.into_iter().filter(|shell| shell.found()) {
        let outside = [("NGTERM_SESSION_ID", "")];
        assert_eq!(wrapped(shell, "-m o3", &outside)?, "-m o3", "{shell:?}");
    }
    Ok(())
}

#[test]
fn a_codex_function_of_the_users_is_left_alone() -> TestResult {
    for shell in Shell::ALL.into_iter().filter(|shell| shell.found()) {
        let script = format!("{}\n{}\ncodex -m o3\n", shell.user_function(), shell.source());
        let run = run_wrapper(shell, &script, &[])?;
        assert_eq!(run.stdout, "user", "{shell:?}");
        assert!(run.calls.is_empty(), "{shell:?}: {:?}", run.calls);
    }
    Ok(())
}

#[test]
fn the_wrapper_keeps_codexs_exit_status() -> TestResult {
    for shell in Shell::ALL.into_iter().filter(|shell| shell.found()) {
        let script = format!("{}\ncodex -m o3\necho {}\n", shell.source(), shell.status());
        let run = run_wrapper(shell, &script, &[("STUB_EXIT", "7")])?;
        assert_eq!(run.stdout, "7", "{shell:?}");
    }
    Ok(())
}

#[test]
fn piped_input_reaches_codex_on_either_path() -> TestResult {
    for shell in Shell::ALL.into_iter().filter(|shell| shell.found()) {
        for (line, expected) in [("-m o3", "--no-daemon -m o3"), ("exec -", "exec -")] {
            let script = format!("{}\necho hello | codex {line}\n", shell.source());
            let run = run_wrapper(shell, &script, &[("STUB_READ_STDIN", "1")])?;
            assert_eq!(run.stdout, "hello", "{shell:?} `codex {line}`");
            assert_eq!(run.calls, [expected], "{shell:?} `codex {line}`");
        }
    }
    Ok(())
}

/// PowerShell hands an unquoted `a,b` to a function as an array, which `@args` alone would split.
#[test]
fn powershell_keeps_a_comma_in_one_argument() -> TestResult {
    if Shell::Pwsh.found() {
        assert_eq!(
            wrapped(Shell::Pwsh, "-c foo=a,b 'a prompt'", &[])?,
            "--no-daemon -c foo=a,b a prompt"
        );
        assert_eq!(wrapped(Shell::Pwsh, "exec --add-dir a,b", &[])?, "exec --add-dir a,b");
    }
    Ok(())
}

/// A function cannot fail its caller's `$?`, so the wrapper tells the prompt, whose D mark would otherwise read 0; the
/// failure is spent there, and the next command's mark is its own.
#[test]
fn powershell_prompt_reports_a_failed_codex() -> TestResult {
    if Shell::Pwsh.found() {
        let script = format!(
            "{}\n\
             $global:__NgtermIntegration.Running = $true\n\
             codex -m o3\n\
             prompt\n\
             $global:__NgtermIntegration.Running = $true\n\
             prompt\n",
            Shell::Pwsh.source()
        );
        let run = run_wrapper(Shell::Pwsh, &script, &[("STUB_EXIT", "7")])?;
        assert!(run.stdout.contains("133;D;7"), "{:?}", run.stdout);
        assert_eq!(run.stdout.matches("133;D;7").count(), 1, "{:?}", run.stdout);
    }
    Ok(())
}
