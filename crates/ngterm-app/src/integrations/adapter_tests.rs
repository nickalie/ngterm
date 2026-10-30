//! The Claude adapter script run for real: what it forwards to the shared wrapper for a hook's argv and payload. A
//! stub wrapper records the forwarded argv; nothing reaches an app. The POSIX script runs on Unix, its PowerShell form
//! (`ngterm-claude-status.ps1`) on Windows, under Windows PowerShell and, where it is installed, PowerShell 7.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const IN_FLIGHT: &str =
    r#"[{"id":"b1","type":"shell","status":"running","description":"cargo test","command":"cargo test"}]"#;

/// A hook payload as Claude Code writes it: compact JSON, the common fields first.
fn payload(event: &str, message: &str, tasks: &str) -> String {
    format!(
        r#"{{"session_id":"s","hook_event_name":"{event}","stop_hook_active":false,"last_assistant_message":"{message}","background_tasks":{tasks},"session_crons":[]}}"#
    )
}

fn resources() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/agent-status")
}

/// The argv the adapter hands the wrapper for `args` with `payload` on stdin. The adapter starts only once its
/// launching shell is reaped, so it is an orphan whose ancestor walk never meets the agent that may be running these
/// tests (a second agent above the hook would make it a silent worker).
#[cfg(unix)]
fn forwarded(args: &[&str], payload: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let stub = dir.path().join("wrapper.sh");
    fs::write(
        &stub,
        "#!/bin/sh\nprintf '%s' \"$*\" > \"$OUT.tmp\" && mv \"$OUT.tmp\" \"$OUT\"\n",
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    let out = dir.path().join("argv");
    let input = dir.path().join("payload.json");
    fs::write(&input, payload).unwrap();
    let launch = r#"(while kill -0 $$ 2>/dev/null; do :; done; exec bash "$0" "$@" <"$PAYLOAD" >/dev/null 2>&1) &"#;
    let status = Command::new("sh")
        .args(["-c", launch])
        .arg(resources().join("ngterm-claude-status.sh"))
        .args(args)
        .env("PAYLOAD", &input)
        .env("OUT", &out)
        .env("NGTERM_STATUS_WRAPPER", &stub)
        .env("NGTERM_SESSION_ID", "S")
        .env_remove("NGTERM_AGENT_RE")
        .status()
        .unwrap();
    assert!(status.success());
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(argv) = fs::read_to_string(&out) {
            return argv;
        }
        assert!(
            Instant::now() < deadline,
            "the adapter never called the wrapper for {args:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The PowerShells a Windows test runs a script under: Windows PowerShell, and PowerShell 7 when it is on `PATH`.
#[cfg(windows)]
pub(super) fn powershells() -> Vec<&'static str> {
    let pwsh = Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-Command", "exit 0"])
        .status()
        .is_ok_and(|status| status.success());
    if pwsh {
        vec!["powershell", "pwsh"]
    } else {
        vec!["powershell"]
    }
}

/// `shell` running the PowerShell script `script` with `args`, as a hook command starts it.
#[cfg(windows)]
pub(super) fn powershell_file(shell: &str, script: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(shell);
    command
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .args(args);
    command
}

/// The argv the adapter hands the wrapper for `args` with `payload` on stdin, the same under each PowerShell. The
/// adapter runs as the test's child: its ancestor walk meets the test, cargo and their shell, no agent on a CI runner
/// (a developer's agent above them counts once, which still reports).
#[cfg(windows)]
fn forwarded(args: &[&str], payload: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let stub = dir.path().join("wrapper.ps1");
    fs::write(
        &stub,
        "Set-Content -LiteralPath $env:OUT -Value ($args -join ' ') -NoNewline\r\n",
    )
    .unwrap();
    let out = dir.path().join("argv");
    let input = dir.path().join("payload.json");
    fs::write(&input, payload).unwrap();
    let mut seen = Vec::new();
    for shell in powershells() {
        let _ = fs::remove_file(&out);
        let status = powershell_file(shell, &resources().join("ngterm-claude-status.ps1"), args)
            .stdin(fs::File::open(&input).unwrap())
            .env("OUT", &out)
            .env("NGTERM_STATUS_WRAPPER", &stub)
            .env("NGTERM_SESSION_ID", "S")
            .env_remove("NGTERM_AGENT_RE")
            .status()
            .unwrap();
        assert!(status.success(), "{shell}: {status}");
        let argv = fs::read_to_string(&out)
            .unwrap_or_else(|err| panic!("{shell}: the adapter never called the wrapper for {args:?}: {err}"));
        seen.push(argv);
    }
    seen.dedup();
    assert_eq!(seen.len(), 1, "the PowerShells differ: {seen:?}");
    seen.remove(0)
}

#[test]
fn a_stop_with_background_work_in_flight_stays_active() {
    let stop = payload("Stop", "started the tests", IN_FLIGHT);
    assert_eq!(forwarded(&["completed", "--auto-reset"], &stop), "active --blink");
    let spaced = stop
        .replace(r#""hook_event_name":"#, r#""hook_event_name": "#)
        .replace(r#""background_tasks":["#, "\"background_tasks\": [\n  ");
    assert_eq!(forwarded(&["completed", "--auto-reset"], &spaced), "active --blink");
}

#[test]
fn a_stop_with_nothing_in_flight_completes() {
    let done = payload("Stop", "all green", "[]");
    assert_eq!(
        forwarded(&["completed", "--auto-reset"], &done),
        "completed --auto-reset"
    );
    // a key quoted in the last message is escaped, so it is not the payload's list
    let quoted = payload("Stop", r#"saw \"background_tasks\":[{\"id\":1}] in the log"#, "[]");
    assert_eq!(
        forwarded(&["completed", "--auto-reset"], &quoted),
        "completed --auto-reset"
    );
    // an older Claude Code sends no list at all
    assert_eq!(
        forwarded(&["completed", "--auto-reset"], r#"{"hook_event_name":"Stop"}"#),
        "completed --auto-reset"
    );
}

#[test]
fn other_states_and_events_forward_verbatim() {
    let stop = payload("Stop", "started the tests", IN_FLIGHT);
    assert_eq!(forwarded(&["active", "--blink"], &stop), "active --blink");
    assert_eq!(forwarded(&["blocked"], &stop), "blocked");
    let subagent = payload("SubagentStop", "started the tests", IN_FLIGHT);
    assert_eq!(
        forwarded(&["completed", "--auto-reset"], &subagent),
        "completed --auto-reset"
    );
    assert_eq!(forwarded(&["completed", "--auto-reset"], ""), "completed --auto-reset");
}

/// The ancestor walk on Windows, through the process table: copies of `cmd.exe` named as agents stand between a
/// launcher and the adapter, each running the next with `/c`. One agent, or a run of one name, is the pane's agent and
/// reports; a second, different agent above the first makes the hook a worker's, which stays silent. The launcher
/// starts the first agent with `start /b` and exits, so the walk ends there, never reaching an agent that may be
/// running these tests; the outermost agent touches `done` once the chain has finished. The paths go unquoted, since
/// `cmd /c` strips the quotes of a command line that starts with one: the test's directories hold no space.
#[cfg(windows)]
#[test]
fn a_hook_under_a_second_agent_is_a_workers_and_stays_silent() {
    use std::os::windows::process::CommandExt as _;
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
    for agent in ["claude", "codex"] {
        let copy = dir.path().join(format!("{agent}.exe"));
        fs::copy(Path::new(&system).join(r"System32\cmd.exe"), copy).unwrap();
    }
    let stub = dir.path().join("wrapper.ps1");
    fs::write(
        &stub,
        "Set-Content -LiteralPath $env:OUT -Value ($args -join ' ') -NoNewline\r\n",
    )
    .unwrap();
    let (out, done) = (dir.path().join("argv"), dir.path().join("done"));
    let adapter = resources().join("ngterm-claude-status.ps1");
    for path in [dir.path(), adapter.as_path()] {
        assert!(
            !path.display().to_string().contains(' '),
            "a space in {}",
            path.display()
        );
    }
    let hook = format!(
        "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File {} blocked",
        adapter.display()
    );
    let agent = |name: &str| dir.path().join(format!("{name}.exe")).display().to_string();
    let under = |agents: &[&str]| {
        let _ = fs::remove_file(&out);
        let _ = fs::remove_file(&done);
        let line = agents
            .iter()
            .rev()
            .fold(hook.clone(), |inner, name| format!("{} /d /c {inner}", agent(name)));
        // `^&` and `^>` reach the first agent, which runs the chain and then touches `done`
        let status = Command::new("cmd")
            .raw_arg(format!("/d /c start \"\" /b {line} ^& type nul ^> {}", done.display()))
            .env("OUT", &out)
            .env("NGTERM_STATUS_WRAPPER", &stub)
            .env("NGTERM_SESSION_ID", "S")
            .env_remove("NGTERM_AGENT_RE")
            .status()
            .unwrap();
        assert!(status.success(), "{agents:?}: {status}");
        let deadline = Instant::now() + Duration::from_secs(60);
        while !done.exists() {
            assert!(Instant::now() < deadline, "{agents:?}: the chain never finished");
            std::thread::sleep(Duration::from_millis(50));
        }
        fs::read_to_string(&out).ok()
    };
    assert_eq!(under(&["claude"]).as_deref(), Some("blocked"), "the pane's agent");
    assert_eq!(
        under(&["claude", "claude"]).as_deref(),
        Some("blocked"),
        "a run of one name"
    );
    assert_eq!(under(&["claude", "codex"]), None, "a worker under another agent");
}
