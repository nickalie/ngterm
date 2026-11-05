//! `ngtermctl agent hook`, built, against an in-process control server over the fake main loop: what it reads on
//! stdin, the `agent.report` it sends, its failures, and, end to end through the real Codex adapter and status wrapper
//! (`resources/agent-status`) into a core, that a call Codex's Auto Review approved never shows as a live approval.
//! Every run's environment is the test's own: no `NGTERM_*`, `CLAUDE*` or `CODEX*` variable of the caller's.

mod support;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ngterm_host::fakes::FakeMainLoop;
use ngterm_proto::Response;
use serde_json::{Value, json};
use support::{
    NGTERMCTL, Server, TestResult, fixtures, isolated, request, serve_core, serve_on, session_id, short_dir,
};

/// A server that answers every request with one result, counting them and keeping the last.
struct Recording {
    _server: Server<()>,
    count: Arc<AtomicUsize>,
    last: Arc<Mutex<Value>>,
}

impl Recording {
    fn serve(socket: &Path, result: Value) -> TestResult<Self> {
        let count = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(Value::Null));
        let (seen, kept) = (Arc::clone(&count), Arc::clone(&last));
        let server = serve_on(socket, FakeMainLoop::spawn(())?, move |(), request| {
            seen.fetch_add(1, Ordering::SeqCst);
            if let Ok(mut last) = kept.lock() {
                *last = serde_json::to_value(&request).unwrap_or_default();
            }
            Response::with_result(result.clone()).into()
        })?;
        Ok(Self {
            _server: server,
            count,
            last,
        })
    }

    fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    fn last(&self) -> TestResult<Value> {
        Ok(self.last.lock().map_err(|_| "poisoned")?.clone())
    }
}

/// `agent.get`'s `agent` for the session's left pane, or the refusal.
fn agent_get(socket: &Path, session: &str) -> TestResult<Result<Value, String>> {
    let response = request(
        socket,
        &json!({"cmd": "agent.get", "target": session, "args": {"pane": "left"}}),
    )?;
    Ok(if response.ok {
        Ok(response.result.ok_or("no result")?["agent"].clone())
    } else {
        Err(response.error.unwrap_or_default())
    })
}

/// Runs `ngtermctl args` with `stdin` piped in.
fn hook(root: &Path, args: &[&str], stdin: &str) -> TestResult<Output> {
    let mut child = isolated(NGTERMCTL, root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut input) = child.stdin.take() {
        // a refusal may come before the read
        let _ = input.write_all(stdin.as_bytes());
    }
    Ok(child.wait_with_output()?)
}

fn outcome(output: &Output) -> (Option<i32>, String, String) {
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A recorded hook payload (`ngterm-core`'s `tests/fixtures/agent_chat`).
fn fixture(path: &str) -> TestResult<String> {
    Ok(std::fs::read_to_string(fixtures().join(path))?)
}

const CLAUDE_ID: &str = "f559ab9f-a5bd-4b2f-91e5-72590d662bf4";
const CLAUDE_TRANSCRIPT: &str =
    "/home/user/.claude/projects/-home-user-project/f559ab9f-a5bd-4b2f-91e5-72590d662bf4.jsonl";

#[test]
fn hook_sends_the_payloads_report_addressed_as_the_status_hooks_address_theirs() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let server = Recording::serve(&socket, json!({"id": "S"}))?;
    let socket_arg = socket.to_str().ok_or("a UTF-8 path")?;
    let args = [
        "agent",
        "hook",
        "claude",
        "SessionStart",
        "--target",
        "S",
        "--pane",
        "right",
        "--pane-id",
        "P",
        "--socket",
        socket_arg,
    ];
    let start = fixture("claude/hooks/01-SessionStart.json")?;
    assert_eq!(
        outcome(&hook(dir.path(), &args, &start)?),
        (Some(0), String::new(), String::new()),
        "a hook prints nothing"
    );
    assert_eq!(server.count(), 1);
    let report = json!({"agent": "claude", "event": "SessionStart", "transcriptPath": CLAUDE_TRANSCRIPT, "conversationId": CLAUDE_ID});
    assert_eq!(
        server.last()?,
        json!({"cmd": "agent.report", "target": "S", "args": {"pane": "right", "paneID": "P", "report": report}})
    );

    let permission = fixture("claude/hooks/33-PermissionRequest.json")?;
    let args = [
        "agent",
        "hook",
        "claude",
        "PermissionRequest",
        "--socket",
        socket_arg,
        "--json",
    ];
    let (code, stdout, stderr) = outcome(&hook(dir.path(), &args, &permission)?);
    assert_eq!(
        (code, stdout.as_str(), stderr.as_str()),
        (Some(0), "{\"ok\":true,\"result\":{\"id\":\"S\"}}\n", "")
    );
    let sent = server.last()?;
    assert_eq!(sent["target"], "active");
    assert_eq!(
        sent["args"]["report"]["approval"],
        json!({"tool": "Bash", "title": "Create empty file allowed.txt", "detail": "touch allowed.txt", "cwd": "/home/user/project"})
    );
    Ok(())
}

/// What no agent writes, or what `agent hook` cannot use, fails with exit 1 and sends nothing: an empty payload, one
/// that is not a JSON object, one missing a field the event needs, a transcript outside the agent's tree.
#[test]
fn a_payload_it_cannot_use_fails_with_exit_1_and_sends_nothing() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let server = Recording::serve(&socket, json!({"id": "S"}))?;
    let socket_arg = socket.to_str().ok_or("a UTF-8 path")?;
    let cases = [
        ("", "no hook payload on standard input"),
        ("\n  \n", "no hook payload on standard input"),
        ("not json", "hook payload is not a JSON object"),
        ("[1,2]", "hook payload is not a JSON object"),
        (r#"{"session_id":"s"}"#, "hook payload has no transcript_path"),
        (
            r#"{"session_id":"s","transcript_path":"/etc/passwd.jsonl"}"#,
            "transcript path is outside a Claude projects/ tree: /etc/passwd.jsonl",
        ),
    ];
    for (payload, message) in cases {
        let args = ["agent", "hook", "claude", "UserPromptSubmit", "--socket", socket_arg];
        assert_eq!(
            outcome(&hook(dir.path(), &args, payload)?),
            (Some(1), String::new(), format!("Error: {message}\n")),
            "{payload:?}"
        );
    }
    assert_eq!(server.count(), 0);
    Ok(())
}

/// A payload with nothing for the record exits 0 without a request: Codex's memory thread (a null transcript), a
/// subagent's prompt.
#[test]
fn a_payload_with_nothing_for_the_record_sends_nothing() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let server = Recording::serve(&socket, json!({"id": "S"}))?;
    let socket_arg = socket.to_str().ok_or("a UTF-8 path")?;
    let memory = r#"{"session_id":"m","transcript_path":null,"hook_event_name":"SessionStart"}"#;
    let subagent = fixture("codex/hooks/main/18-UserPromptSubmit.json")?;
    for (event, payload) in [("SessionStart", memory), ("UserPromptSubmit", subagent.as_str())] {
        let args = ["agent", "hook", "codex", event, "--socket", socket_arg];
        assert_eq!(
            outcome(&hook(dir.path(), &args, payload)?),
            (Some(0), String::new(), String::new()),
            "{event}"
        );
    }
    assert_eq!(server.count(), 0);
    Ok(())
}

#[test]
fn a_missing_socket_or_a_refusal_fails_with_exit_1() -> TestResult {
    let dir = short_dir()?;
    let missing = dir.path().join("missing.sock");
    let start = fixture("claude/hooks/01-SessionStart.json")?;
    let args = [
        "agent",
        "hook",
        "claude",
        "SessionStart",
        "--socket",
        missing.to_str().ok_or("a UTF-8 path")?,
    ];
    let (code, stdout, stderr) = outcome(&hook(dir.path(), &args, &start)?);
    assert_eq!((code, stdout.as_str()), (Some(1), ""));
    assert!(
        stderr.starts_with("Error: ") && stderr.contains("missing.sock"),
        "{stderr}"
    );

    let socket = dir.path().join("ngterm.sock");
    let _server = serve_core(&socket)?;
    let args = [
        "agent",
        "hook",
        "claude",
        "SessionStart",
        "--target",
        "zzz",
        "--socket",
        socket.to_str().ok_or("a UTF-8 path")?,
    ];
    assert_eq!(
        outcome(&hook(dir.path(), &args, &start)?),
        (Some(1), String::new(), "error: no such session: zzz\n".to_owned())
    );
    Ok(())
}

#[test]
fn an_unknown_agent_or_event_is_a_usage_error() -> TestResult {
    let dir = short_dir()?;
    for (args, message) in [
        (
            ["agent", "hook", "gemini", "SessionStart"],
            "unknown agent: gemini (claude|codex)",
        ),
        (
            ["agent", "hook", "codex", "Stop"],
            "unknown agent event: Stop (SessionStart|UserPromptSubmit|PermissionRequest|SessionEnd)",
        ),
    ] {
        let (code, stdout, stderr) = outcome(&hook(dir.path(), &args, "{}")?);
        assert_eq!((code, stdout.as_str()), (Some(64), ""), "{args:?}");
        assert!(stderr.starts_with(&format!("Error: {message}\n")), "{stderr}");
    }
    Ok(())
}

/// Through a core: a start sets the pane's record, a permission request's candidate is live only while the pane is
/// blocked, and the conversation's end clears the record.
#[test]
fn a_core_keeps_what_the_hooks_report() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let _server = serve_core(&socket)?;
    let session = session_id(&socket)?;
    let socket_arg = socket.to_str().ok_or("a UTF-8 path")?;
    let send = |event: &str, path: &str| -> TestResult {
        let args = [
            "agent", "hook", "claude", event, "--target", &session, "--pane", "left", "--socket", socket_arg,
        ];
        let output = hook(dir.path(), &args, &fixture(path)?)?;
        assert_eq!(outcome(&output), (Some(0), String::new(), String::new()), "{event}");
        Ok(())
    };
    assert_eq!(agent_get(&socket, &session)?, Err("no agent in this pane".to_owned()));
    send("SessionStart", "claude/hooks/01-SessionStart.json")?;
    let agent = agent_get(&socket, &session)?.map_err(|err| err.clone())?;
    assert_eq!(
        (&agent["agent"], &agent["conversationId"], &agent["transcriptPath"]),
        (&json!("claude"), &json!(CLAUDE_ID), &json!(CLAUDE_TRANSCRIPT))
    );
    send("PermissionRequest", "claude/hooks/33-PermissionRequest.json")?;
    assert_eq!(
        agent_get(&socket, &session)?
            .map_err(|err| err.clone())?
            .get("approval"),
        None
    );
    let blocked = request(
        &socket,
        &json!({"cmd": "session.status", "target": session, "args": {"status": "blocked", "pane": "left"}}),
    )?;
    assert!(blocked.ok, "{blocked:?}");
    let agent = agent_get(&socket, &session)?.map_err(|err| err.clone())?;
    assert_eq!(agent["approval"]["detail"], "touch allowed.txt");
    send("SessionEnd", "claude/hooks/59-SessionEnd.json")?;
    assert_eq!(agent_get(&socket, &session)?, Err("no agent in this pane".to_owned()));
    Ok(())
}

/// The bash the Codex adapter runs under: `bash` on Unix, Git for Windows' on Windows (as Codex runs it there).
fn codex_bash() -> Option<PathBuf> {
    if cfg!(unix) {
        return Some(PathBuf::from("bash"));
    }
    ["ProgramFiles", "ProgramW6432"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|dir| PathBuf::from(dir).join("Git").join("bin").join("bash.exe"))
        .find(|bash| bash.is_file())
}

/// `path` as bash finds a program: forward slashes, which Git for Windows' bash reads as Windows paths do.
fn bash_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// End to end: the Codex adapter, the real status wrapper and this `ngtermctl` into a core, over the recorded Auto
/// Review session (its `PermissionRequest` fired, the guardian approved with no dialog, the command ran). The candidate
/// is stored but never live, not even when a later block comes; a candidate the user must answer (the watcher reports
/// `blocked` for it) is. The watcher makes no check here: it reads a screen, which this core has none of.
#[test]
fn a_call_auto_review_approved_never_yields_a_live_approval() -> TestResult {
    let Some(bash) = codex_bash() else {
        eprintln!("SKIPPED: no Git for Windows bash");
        return Ok(());
    };
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let _server = serve_core(&socket)?;
    let session = session_id(&socket)?;
    let adapter = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/agent-status/ngterm-codex-status.sh");
    let codex = |action: &str, path: &str| -> TestResult {
        let mut child = isolated(&bash, dir.path())
            .arg(bash_path(&adapter))
            .arg(action)
            .env("NGTERMCTL", bash_path(Path::new(NGTERMCTL)))
            .env("NGTERM_SESSION_ID", &session)
            .env("NGTERM_SOCKET", &socket)
            .env("NGTERM_PANE", "left")
            .env("NGTERM_CODEX_WATCH_FILE", bash_path(&dir.path().join("watch")))
            .env("NGTERM_CODEX_WATCH_MAX_CHECKS", "0")
            .env("TMPDIR", bash_path(dir.path()))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        if let Some(mut input) = child.stdin.take() {
            let _ = input.write_all(fixture(path)?.as_bytes());
        }
        assert!(child.wait()?.success(), "{action}");
        Ok(())
    };
    let status = |state: &str| -> TestResult {
        let set = request(
            &socket,
            &json!({"cmd": "session.status", "target": session, "args": {"status": state, "pane": "left"}}),
        )?;
        assert!(set.ok, "{set:?}");
        Ok(())
    };
    let approval = || -> TestResult<Value> {
        let agent = agent_get(&socket, &session)?.map_err(|err| err.clone())?;
        Ok(agent.get("approval").cloned().unwrap_or(Value::Null))
    };
    let run = "codex/hooks/auto-review";
    codex("session-start", &format!("{run}/01-SessionStart.json"))?;
    codex("user-prompt-submit", &format!("{run}/02-UserPromptSubmit.json"))?;
    codex("pre-tool-use", &format!("{run}/03-PreToolUse.json"))?;
    codex("permission-request", &format!("{run}/04-PermissionRequest.json"))?;
    let agent = agent_get(&socket, &session)?.map_err(|err| err.clone())?;
    assert_eq!(agent["conversationId"], "01a1a305-0669-7522-9a3a-98fa4f56da28");
    assert_eq!(approval()?, Value::Null, "a candidate, not a dialog");
    codex("post-tool-use", &format!("{run}/05-PostToolUse.json"))?;
    codex("stop", &format!("{run}/06-Stop.json"))?;
    status("blocked")?;
    assert_eq!(approval()?, Value::Null, "a later block does not revive it");

    // the control: a prompt the user must answer goes live once the watcher reports it
    status("active")?;
    codex("permission-request", &format!("{run}/04-PermissionRequest.json"))?;
    status("blocked")?;
    assert_eq!(approval()?["detail"], "curl -sI https://example.com");
    Ok(())
}
