//! The restore adapter run for real: which `ngtermctl session restore` calls it makes for a hook's event, payload and
//! environment, and the `ngtermctl agent hook` calls that keep the pane's agent record (the Android chat view) on the
//! same conversation. A stub `ngtermctl` records each call's argv, and an `agent` call's stdin; no `NGTERM_*` variable
//! of a live instance reaches the script and nothing talks to an app. The POSIX script runs on Unix under `bash`; its PowerShell form
//! (`ngterm-agent-restore.ps1`) on Windows, under Windows PowerShell and, where it is installed, PowerShell 7, each
//! run asserted to make the same calls.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const ID: &str = "0f8a4c3e-6b1d-4e2a-9c7f-1a2b3c4d5e6f";
const SESSION: &str = "06882D55-AC80-4B1B-8BBE-682FDDDEC95A";
const PANE: &str = "05E0AB65-271B-4389-9946-C3A7E2060CAC";
const SOCKET: &str = "/nonexistent/ngterm-restore-test.sock";

fn resources() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/agent-status")
}

/// The pin a start writes: shell code for the pane's login shell, POSIX `||` on Unix, a form Windows PowerShell 5.1
/// (no `||`) takes on Windows.
fn resume_line() -> String {
    if cfg!(windows) {
        format!("claude --resume {ID}; if (-not $?) {{ claude }}")
    } else {
        format!("claude --resume {ID} || claude")
    }
}

/// A Claude Code hook payload as it writes one: compact JSON, the common fields first.
fn claude_payload(event: &str, extra: &str) -> String {
    format!(
        r#"{{"session_id":"{ID}","transcript_path":"/home/u/.claude/projects/-p/{ID}.jsonl","cwd":"/p","hook_event_name":"{event}"{extra}}}"#
    )
}

/// A scratch directory with a stub `ngtermctl` that appends each call's arguments, tab-separated, to `calls`, and
/// writes an `agent` call's stdin over `agent-stdin`.
struct Bench {
    dir: tempfile::TempDir,
}

impl Bench {
    #[cfg(unix)]
    const STUB: (&str, &str) = (
        "ngtermctl",
        "#!/usr/bin/env bash\ndir=$(dirname \"$0\")\n[ \"$1\" = agent ] && cat > \"$dir/agent-stdin\"\n\
         (IFS=$'\\t'; printf '%s\\n' \"$*\") >> \"$dir/calls\"\n",
    );
    #[cfg(windows)]
    const STUB: (&str, &str) = (
        "ngtermctl.ps1",
        "if ($args[0] -ceq 'agent') {\r\n\
         [IO.File]::WriteAllText((Join-Path $PSScriptRoot 'agent-stdin'), (@($input) -join \"`n\"))\r\n}\r\n\
         Add-Content -LiteralPath (Join-Path $PSScriptRoot 'calls') -Value ($args -join \"`t\")\r\n",
    );

    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (name, text) = Self::STUB;
        let stub = dir.path().join(name);
        ngterm_host::fakes::write_executable(&stub, text).unwrap();
        Self { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Every call the stub recorded so far, each as its argument list.
    fn all_calls(&self) -> Vec<Vec<String>> {
        fs::read_to_string(self.path("calls"))
            .unwrap_or_default()
            .lines()
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect()
    }

    /// The `session restore` calls the stub recorded so far.
    fn calls(&self) -> Vec<Vec<String>> {
        self.all_calls()
            .into_iter()
            .filter(|call| call.first().is_some_and(|word| word == "session"))
            .collect()
    }

    /// The `agent hook` calls the stub recorded so far.
    fn agent_calls(&self) -> Vec<Vec<String>> {
        self.all_calls()
            .into_iter()
            .filter(|call| call.first().is_some_and(|word| word == "agent"))
            .collect()
    }

    /// What the last `agent` call read on stdin.
    fn agent_stdin(&self) -> String {
        fs::read_to_string(self.path("agent-stdin")).unwrap_or_default()
    }

    /// The adapter with `args` under `shell` (Unix: `bash`), in a pane's environment from scratch: the pane variables,
    /// the stub, a private TMPDIR, plus `vars`, less `removed`.
    fn command(&self, shell: &str, vars: &[(&str, &str)], removed: &[&str], args: &[&str]) -> Command {
        #[cfg(unix)]
        let mut command = {
            let mut command = Command::new(shell);
            command
                .arg(resources().join("ngterm-agent-restore.sh"))
                .args(args)
                .env_clear()
                .env("PATH", std::env::var("PATH").unwrap_or_default());
            command
        };
        #[cfg(windows)]
        let mut command = {
            let script = resources().join("ngterm-agent-restore.ps1");
            let mut command = super::adapter_tests::powershell_file(shell, &script, args);
            // the system's variables PowerShell needs stay; the live instance's and the agent's go
            for (key, _) in std::env::vars_os() {
                let key = key.to_string_lossy().into_owned();
                if ["NGTERM_", "AGTERM_", "CLAUDE"]
                    .iter()
                    .any(|prefix| key.starts_with(prefix))
                {
                    command.env_remove(key);
                }
            }
            command
        };
        command
            .env("NGTERMCTL", self.path(Self::STUB.0))
            .env("TMPDIR", self.dir.path())
            .env("NGTERM_SESSION_ID", SESSION)
            .env("NGTERM_PANE_ID", PANE)
            .env("NGTERM_SOCKET", SOCKET);
        for (key, value) in vars {
            command.env(key, value);
        }
        for key in removed {
            command.env_remove(key);
        }
        command
    }

    /// The shells the adapter runs under.
    #[cfg(unix)]
    fn shells() -> Vec<&'static str> {
        vec!["bash"]
    }

    #[cfg(windows)]
    fn shells() -> Vec<&'static str> {
        super::adapter_tests::powershells()
    }

    /// Runs the adapter with `args` and `payload` on stdin under each shell, which must make the same calls; they are
    /// recorded once.
    fn run(&self, vars: &[(&str, &str)], removed: &[&str], args: &[&str], payload: &str) {
        let before = self.all_calls();
        let mut made: Option<Vec<Vec<String>>> = None;
        for shell in Self::shells() {
            Self::run_one(self.command(shell, vars, removed, args), payload);
            let calls = self.all_calls()[before.len()..].to_vec();
            match &made {
                Some(made) => assert_eq!(&calls, made, "{shell} differs, for {args:?} with {payload}"),
                None => made = Some(calls),
            }
            let lines: Vec<String> = before.iter().map(|call| call.join("\t") + "\n").collect();
            fs::write(self.path("calls"), lines.concat()).unwrap();
        }
        let lines: Vec<String> = before
            .iter()
            .chain(made.iter().flatten())
            .map(|call| call.join("\t") + "\n")
            .collect();
        fs::write(self.path("calls"), lines.concat()).unwrap();
    }

    fn run_one(mut command: Command, payload: &str) -> Output {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        {
            use std::io::Write;
            let mut stdin = child.stdin.take().unwrap();
            // the adapter may leave before reading (outside ngterm it exits first), as a hook may: the agent's write
            // then meets a broken pipe, which is no failure; what the adapter did shows in the calls it made
            if let Err(err) = stdin.write_all(payload.as_bytes()) {
                assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe, "writing the payload: {err}");
            }
        }
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "the hook exits 0: {output:?}");
        assert!(
            output.stdout.is_empty() && output.stderr.is_empty(),
            "the hook is silent: {output:?}"
        );
        output
    }

    /// The adapter as Claude Code runs it: a child of the agent, `vars` on top of the pane's environment.
    fn claude(&self, event: &str, payload: &str, vars: &[(&str, &str)]) -> Vec<Vec<String>> {
        self.run(vars, &[], &["claude", event], payload);
        self.calls()
    }
}

fn pin(line: &str) -> Vec<String> {
    [
        "session",
        "restore",
        line,
        "--target",
        SESSION,
        "--pane-id",
        PANE,
        "--socket",
        SOCKET,
    ]
    .map(str::to_owned)
    .to_vec()
}

fn clear() -> Vec<String> {
    pin("--clear")
}

const CLI: (&str, &str) = ("CLAUDE_CODE_ENTRYPOINT", "cli");

#[cfg(unix)]
#[test]
fn ngswap_restore_keeps_the_account_on_resume_and_fallback() {
    let bench = Bench::new();
    let helper = bench.path("profile helper.py");
    fs::write(
        &helper,
        "import shlex,sys\nprint(shlex.join(['pinned-ngswap', 'work-account', *sys.argv[1:]]))\n",
    )
    .unwrap();
    let vars = [
        CLI,
        ("NGSWAP_RUN_PROVIDER", "claude"),
        ("NGSWAP_SESSION_HELPER", helper.to_str().unwrap()),
    ];
    let calls = bench.claude("start", &claude_payload("SessionStart", ""), &vars);
    assert_eq!(
        calls,
        vec![pin(&format!(
            "pinned-ngswap work-account prepare --provider claude --slot {SESSION}:{PANE}:resume -- --resume {ID} || pinned-ngswap work-account prepare --provider claude --slot {SESSION}:{PANE}:fresh"
        ))]
    );
}

#[cfg(unix)]
#[test]
fn ngswap_restore_preparation_failure_keeps_the_existing_pin() {
    for marker in ["NGSWAP_RUN_PROVIDER", "NGSWAP_DEFAULT_ROOT"] {
        for fail_fresh_only in [false, true] {
            let bench = Bench::new();
            let helper = bench.path("failing-helper.py");
            let code = if fail_fresh_only {
                "import sys\nif '--resume' not in sys.argv: sys.exit(1)\nprint('resume-command')\n"
            } else {
                "import sys\nsys.exit(1)\n"
            };
            fs::write(&helper, code).unwrap();
            assert!(
                bench
                    .claude(
                        "start",
                        &claude_payload("SessionStart", ""),
                        &[
                            CLI,
                            (marker, "claude"),
                            ("NGSWAP_SESSION_HELPER", helper.to_str().unwrap()),
                        ],
                    )
                    .is_empty()
            );
            assert_eq!(bench.agent_calls().len(), 1);
        }
    }
}

#[cfg(unix)]
#[test]
fn another_ngswap_provider_does_not_suppress_claude_restore() {
    let bench = Bench::new();
    let calls = bench.claude(
        "start",
        &claude_payload("SessionStart", ""),
        &[CLI, ("NGSWAP_RUN_PROVIDER", "codex"), ("NGSWAP_DEFAULT_ROOT", "/store")],
    );
    assert_eq!(calls, vec![pin(&resume_line())]);
}

#[test]
fn a_claude_start_pins_the_live_conversation() {
    let bench = Bench::new();
    for source in ["startup", "resume", "clear", "compact"] {
        let payload = claude_payload("SessionStart", &format!(r#","source":"{source}","model":"m""#));
        bench.claude("start", &payload, &[CLI]);
    }
    assert_eq!(bench.calls(), vec![pin(&resume_line()); 4]);
}

#[test]
fn a_claude_start_without_a_pane_id_or_socket_targets_the_session_alone() {
    let bench = Bench::new();
    bench.run(
        &[CLI],
        &["NGTERM_PANE_ID", "NGTERM_SOCKET"],
        &["claude", "start"],
        &claude_payload("SessionStart", ""),
    );
    let line = resume_line();
    assert_eq!(
        bench.calls(),
        [["session", "restore", line.as_str(), "--target", SESSION]
            .map(str::to_owned)
            .to_vec()]
    );
}

/// The pin is shell code typed into the login shell: anything but a UUID-shaped hex id leaves the pin alone.
#[test]
fn a_claude_start_with_a_bad_id_pins_nothing() {
    let bench = Bench::new();
    let bad = [
        String::new(),
        r#"{"hook_event_name":"SessionStart"}"#.to_owned(),
        r#"{"session_id":"","hook_event_name":"SessionStart"}"#.to_owned(),
        r#"{"session_id":"abc123","hook_event_name":"SessionStart"}"#.to_owned(),
        r#"{"session_id":"0f8a4c3e-6b1d-4e2a-9c7f-1a2b3c4d5e6g","hook_event_name":"SessionStart"}"#.to_owned(),
        r#"{"session_id":"0f8a4c3e-6b1d-4e2a-9c7f-1a2b3c4d5e6f; rm -rf ~","hook_event_name":"SessionStart"}"#
            .to_owned(),
        r#"{"session_id":"$(touch /tmp/x)","hook_event_name":"SessionStart"}"#.to_owned(),
        r#"{"session_id":"served:0f8a4c3e-6b1d-4e2a-9c7f-1a2b3c4d5e6f","hook_event_name":"SessionStart"}"#.to_owned(),
        r#"{"session_id":"0f8a4c3e-6b1d-4e2a-9c7f-1a2b3c4d5e6f\" || evil \"","hook_event_name":"SessionStart"}"#
            .to_owned(),
        r#"{"session_id":7,"hook_event_name":"SessionStart"}"#.to_owned(),
        // an id quoted inside a string value is escaped, so it is not the payload's
        format!(r#"{{"last":"\"session_id\":\"{ID}\"","hook_event_name":"SessionStart"}}"#),
    ];
    for payload in &bad {
        assert_eq!(
            bench.claude("start", payload, &[CLI]),
            Vec::<Vec<String>>::new(),
            "{payload}"
        );
    }
}

/// `claude -p` (spawned by a tool call of the pane's claude, it inherits `cli` and Claude Code rewrites it to
/// `sdk-cli` before any hook), Agent SDK runs and subagents never pin over the interactive conversation.
#[test]
fn a_non_interactive_claude_or_a_subagent_pins_nothing() {
    let bench = Bench::new();
    let start = claude_payload("SessionStart", r#","source":"startup""#);
    for entrypoint in ["sdk-cli", "sdk-ts", "sdk-py", "mcp", "claude-vscode", ""] {
        bench.claude("start", &start, &[("CLAUDE_CODE_ENTRYPOINT", entrypoint)]);
    }
    bench.claude("start", &start, &[]);
    let subagent = claude_payload("SessionStart", r#","agent_id":"a1b2","agent_type":"Explore""#);
    bench.claude("start", &subagent, &[CLI]);
    let exit = claude_payload("SessionEnd", r#","reason":"prompt_input_exit""#);
    bench.claude("end", &exit, &[("CLAUDE_CODE_ENTRYPOINT", "sdk-cli")]);
    assert_eq!(bench.calls(), Vec::<Vec<String>>::new());
}

#[test]
fn a_claude_end_clears_the_pin_only_on_a_deliberate_exit() {
    let bench = Bench::new();
    for reason in ["other", "clear", "resume", "bypass_permissions_disabled"] {
        let payload = claude_payload("SessionEnd", &format!(r#","reason":"{reason}""#));
        assert_eq!(
            bench.claude("end", &payload, &[CLI]),
            Vec::<Vec<String>>::new(),
            "{reason}"
        );
    }
    assert_eq!(
        bench.claude("end", &claude_payload("SessionEnd", ""), &[CLI]),
        Vec::<Vec<String>>::new()
    );
    for reason in ["prompt_input_exit", "logout"] {
        let payload = claude_payload("SessionEnd", &format!(r#","reason":"{reason}""#));
        bench.claude("end", &payload, &[CLI]);
    }
    assert_eq!(bench.calls(), [clear(), clear()]);
}

#[test]
fn outside_ngterm_the_adapter_does_nothing() {
    let bench = Bench::new();
    for (agent, event, payload) in [
        ("claude", "start", claude_payload("SessionStart", "")),
        ("claude", "end", claude_payload("SessionEnd", r#","reason":"logout""#)),
    ] {
        bench.run(&[CLI], &["NGTERM_SESSION_ID"], &[agent, event], &payload);
    }
    bench.run(&[CLI], &[], &[], "");
    assert_eq!(bench.all_calls(), Vec::<Vec<String>>::new());
}

/// The `agent hook` call the adapter makes for `event`, addressed as the status hooks address theirs.
fn agent_hook(event: &str) -> Vec<String> {
    [
        "agent",
        "hook",
        "claude",
        event,
        "--target",
        SESSION,
        "--pane",
        "left",
        "--pane-id",
        PANE,
        "--socket",
        SOCKET,
    ]
    .map(str::to_owned)
    .to_vec()
}

/// Every start and every end, whatever its reason, hands the payload to `agent hook`: the start sets the pane's agent
/// record, the end clears it when it is the record's conversation (the core checks that).
#[test]
fn a_claude_start_and_any_end_pipe_their_payload_to_agent_hook() {
    let bench = Bench::new();
    let pane = [CLI, ("NGTERM_PANE", "left")];
    let start = claude_payload("SessionStart", r#","source":"clear""#);
    bench.claude("start", &start, &pane);
    assert_eq!(bench.agent_calls(), [agent_hook("SessionStart")]);
    assert_eq!(bench.agent_stdin().trim_end(), start);
    for reason in ["clear", "other", "prompt_input_exit"] {
        let end = claude_payload("SessionEnd", &format!(r#","reason":"{reason}""#));
        bench.claude("end", &end, &pane);
        assert_eq!(bench.agent_stdin().trim_end(), end, "{reason}");
    }
    assert_eq!(bench.agent_calls()[1..], vec![agent_hook("SessionEnd"); 3]);
    // the record follows a conversation whatever its id looks like: the pin's strict check is the pin's alone
    bench.claude(
        "start",
        r#"{"session_id":"abc123","hook_event_name":"SessionStart"}"#,
        &pane,
    );
    assert_eq!(bench.agent_calls().len(), 5);
}

/// The guards the pin has hold for the agent record too: a recorded `claude -p` worker's start, an SDK run and a
/// subagent make no call.
#[test]
fn a_worker_or_a_subagent_makes_no_agent_call() {
    let bench = Bench::new();
    let worker = super::adapter_tests::fixture("claude/hooks/53-SessionStart.json");
    bench.claude("start", &worker, &[("CLAUDE_CODE_ENTRYPOINT", "sdk-cli")]);
    bench.claude(
        "end",
        &claude_payload("SessionEnd", ""),
        &[("CLAUDE_CODE_ENTRYPOINT", "sdk-ts")],
    );
    bench.claude("start", &claude_payload("SessionStart", ""), &[]);
    let subagent = claude_payload("SessionStart", r#","agent_id":"a1b2","agent_type":"Explore""#);
    bench.claude("start", &subagent, &[CLI]);
    assert_eq!(bench.all_calls(), Vec::<Vec<String>>::new());
}
