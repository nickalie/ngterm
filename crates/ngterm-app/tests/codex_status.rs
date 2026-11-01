//! The Codex status adapter the agent integrations install (`resources/agent-status/ngterm-codex-status.sh`), run for
//! real under bash with recording stand-ins for the status wrapper (`NGTERM_STATUS_WRAPPER`) and `ngtermctl`
//! (`NGTERMCTL`): which lifecycle events it reports and which it leaves alone.
//!
//! Ported from upstream's `agtermCore/Tests/agtermCoreTests/CodexStatusHookTests.swift` (v0.39.1), in this build's
//! names: `eventWithoutTranscriptLeavesStatusAndWatcherAlone` and `eventWithTranscriptReportsStatus` only. The rest of
//! that suite has no twin yet (`docs/backlog/claude-codex-and-opencode-v2-hook-tests-have-no-rust-twin.md`).
//!
//! Upstream reads the payload's `transcript_path` with macOS's `/usr/bin/plutil`; where that is missing the adapter
//! reads it with `python3`, else `jq`, and with none of the three reports the event (fail open). Each case runs once
//! with only `python3` on `PATH` and once with only `jq` (a missing one prints `SKIPPED:`), on a `PATH` of the tools the
//! adapter needs and nothing else. Where `/usr/bin/plutil` exists (macOS) the adapter reads it first, so both runs
//! check plutil's path instead, and the fail-open case does not apply.
//!
//! The adapter is a POSIX script (on Windows Codex runs it under Git Bash), so these tests are Unix-only.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ngterm_host::fakes::write_executable;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn hook() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/agent-status/ngterm-codex-status.sh")
}

/// The commands the adapter runs for a lifecycle event (and its watcher, which `user-prompt-submit` starts), besides the
/// payload reader under test.
const TOOLS: &[&str] = &["bash", "dirname", "cat", "rm", "tr", "tail", "awk", "sed", "sleep"];

/// Which payload reader the adapter finds on its `PATH`.
#[derive(Clone, Copy, Debug)]
enum Reader {
    Python3,
    Jq,
    /// Neither: the adapter cannot read the payload.
    None,
}

impl Reader {
    fn tool(self) -> Option<&'static str> {
        match self {
            Self::Python3 => Some("python3"),
            Self::Jq => Some("jq"),
            Self::None => None,
        }
    }
}

/// `name` on this process's `PATH`.
fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

/// What one adapter run left behind.
struct HookRun {
    status_calls: Vec<String>,
    token: Option<String>,
    exit: Option<i32>,
}

fn lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Runs the adapter for `action` with `input` on stdin and only `reader` of the payload readers on its `PATH`; `None`
/// after printing why when that reader is not installed here.
fn run(action: &str, input: &str, reader: Reader) -> TestResult<Option<HookRun>> {
    let dir = tempfile::tempdir()?;
    let bin = dir.path().join("bin");
    fs::create_dir(&bin)?;
    for tool in TOOLS.iter().copied().chain(reader.tool()) {
        let Some(path) = which(tool) else {
            eprintln!("SKIPPED: {action} with {reader:?}: no {tool} on PATH");
            return Ok(None);
        };
        std::os::unix::fs::symlink(path, bin.join(tool))?;
    }
    let statuses = dir.path().join("statuses");
    let controls = dir.path().join("controls");
    let token_file = dir.path().join("watch-token");
    fs::write(&token_file, "token\n")?;
    let status_wrapper = dir.path().join("status-wrapper");
    write_executable(
        &status_wrapper,
        &format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", statuses.display()),
    )?;
    let ngtermctl = dir.path().join("ngtermctl");
    write_executable(
        &ngtermctl,
        &format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", controls.display()),
    )?;

    let mut child = Command::new(bin.join("bash"))
        .arg(hook())
        .arg(action)
        .env_clear()
        .env("PATH", &bin)
        .env("NGTERMCTL", &ngtermctl)
        .env("NGTERM_STATUS_WRAPPER", &status_wrapper)
        .env("NGTERM_SESSION_ID", "sid")
        .env("NGTERM_SOCKET", dir.path().join("ngterm.sock"))
        .env("NGTERM_PANE", "right")
        .env("NGTERM_PANE_ID", "stable-token")
        .env("NGTERM_CODEX_WATCH_FILE", &token_file)
        .env("NGTERM_CODEX_WATCH_MAX_CHECKS", "0")
        .env("NGTERM_CODEX_WATCH_INTERVAL", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    {
        use std::io::Write as _;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        // the adapter may exit without reading it all
        let _ = stdin.write_all(input.as_bytes());
    }
    let status = child.wait()?;
    Ok(Some(HookRun {
        status_calls: lines(&statuses),
        token: fs::read_to_string(&token_file).ok(),
        exit: status.code(),
    }))
}

const READERS: [Reader; 2] = [Reader::Python3, Reader::Jq];

const TRANSCRIPT_EVENTS: [&str; 4] = ["session-start", "user-prompt-submit", "pre-tool-use", "post-tool-use"];

/// Codex's memory consolidation thread reaches these user hooks but no user Stop, leaving the pane active.
#[test]
fn event_without_transcript_leaves_status_and_watcher_alone() -> TestResult {
    for reader in READERS {
        for action in TRANSCRIPT_EVENTS {
            let Some(result) = run(action, r#"{"hook_event_name":"x","transcript_path":null}"#, reader)? else {
                continue;
            };
            assert!(
                result.status_calls.is_empty(),
                "{action} with {reader:?}: {:?}",
                result.status_calls
            );
            assert_eq!(result.token.as_deref(), Some("token\n"), "{action} with {reader:?}");
            assert_eq!(result.exit, Some(0), "{action} with {reader:?}");
        }
    }
    Ok(())
}

#[test]
fn event_with_transcript_reports_status() -> TestResult {
    let cases = [
        ("session-start", "idle"),
        ("user-prompt-submit", "active --blink"),
        ("pre-tool-use", "active --blink"),
        ("post-tool-use", "active --blink"),
    ];
    let input = r#"{"hook_event_name":"x","transcript_path":"/tmp/rollout.jsonl"}"#;
    for reader in READERS {
        for (action, status) in cases {
            let Some(result) = run(action, input, reader)? else {
                continue;
            };
            assert_eq!(result.status_calls, [status], "{action} with {reader:?}");
        }
    }
    Ok(())
}

/// ngterm's own case: a payload with no `transcript_path` key, or one the adapter cannot read, is reported.
#[test]
fn an_event_the_adapter_cannot_read_as_ephemeral_reports_status() -> TestResult {
    let unreadable = [
        (r#"{"hook_event_name":"x"}"#, READERS.as_slice()),
        ("not json", READERS.as_slice()),
        (r#"{"hook_event_name":"x","transcript_path":null}"#, &[Reader::None]),
    ];
    for (input, readers) in unreadable {
        for &reader in readers {
            if matches!(reader, Reader::None) && Path::new("/usr/bin/plutil").exists() {
                continue;
            }
            let Some(result) = run("pre-tool-use", input, reader)? else {
                continue;
            };
            assert_eq!(result.status_calls, ["active --blink"], "{input} with {reader:?}");
        }
    }
    Ok(())
}
