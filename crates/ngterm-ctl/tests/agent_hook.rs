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
    NGTERMCTL, Server, TestResult, fixtures, isolated, request, serve_core, serve_on, session_id, short_dir, state_dir,
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
    let sent =
        json!({"cmd": "agent.report", "target": "S", "args": {"pane": "right", "paneID": "P", "report": report}});
    assert_eq!(server.last()?, sent);

    // Windows PowerShell 5.1 on a UTF-8 console starts what it pipes to a program with a byte-order mark
    assert_eq!(
        outcome(&hook(dir.path(), &args, &format!("\u{feff}{start}"))?),
        (Some(0), String::new(), String::new()),
        "a byte-order mark before the payload is not part of it"
    );
    assert_eq!(server.count(), 2);
    assert_eq!(server.last()?, sent);

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

/// The transcript of the Claude conversation `id` in a projects tree under `root`.
fn claude_transcript(root: &Path, id: &str) -> TestResult<PathBuf> {
    let project = root.join("home").join(".claude").join("projects").join("-w");
    std::fs::create_dir_all(&project)?;
    Ok(project.join(format!("{id}.jsonl")))
}

/// A Claude Code hook payload of `event` for the conversation `id`.
fn claude_payload(event: &str, id: &str, transcript: &Path) -> String {
    json!({
        "session_id": id,
        "transcript_path": transcript,
        "cwd": "/w",
        "permission_mode": "default",
        "hook_event_name": event,
        "source": "startup",
        "tool_name": "Bash",
        "tool_input": {"command": "true"},
        "background_tasks": [],
    })
    .to_string()
}

/// The seeded session's agent status; null while idle.
fn status_of(socket: &Path) -> TestResult<Value> {
    let tree = request(socket, &json!({"cmd": "tree"}))?;
    Ok(tree.result.ok_or("no tree")?["tree"]["workspaces"][0]["sessions"][0]["status"].clone())
}

const MOVED_FROM: &str = "0a0a0a0a-0000-4000-8000-00000000000a";
const MOVED_TO: &str = "0b0b0b0b-0000-4000-8000-00000000000b";
const NOBODYS: &str = "0c0c0c0c-0000-4000-8000-00000000000c";
const ELSEWHERE: &str = "0d0d0d0d-0000-4000-8000-00000000000d";
/// What Claude Code's daemon was started with: a session and a pane that are gone.
const DAEMONS_SESSION: &str = "5B4EE31F-9B89-472B-9990-C1DCEEF7A966";
const DAEMONS_PANE: &str = "121CAAB2-9079-4F7E-A0CB-808CE743B005";

/// A Claude hook of `resources/agent-status` as Claude Code runs it in a background job of its daemon, for the
/// conversation `id`: `CLAUDE_JOB_DIR` set, the daemon's `NGTERM_*` naming a session that is gone, its socket the
/// instance's.
struct BackgroundJob<'a> {
    bash: &'a Path,
    root: &'a Path,
    socket: &'a Path,
}

impl BackgroundJob<'_> {
    fn hook(&self, script: &str, args: &[&str], event: &str, id: &str) -> TestResult<(Option<i32>, String, String)> {
        let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/agent-status");
        let mut child = isolated(self.bash, self.root)
            .arg(bash_path(&scripts.join(script)))
            .args(args)
            .env("NGTERMCTL", bash_path(Path::new(NGTERMCTL)))
            .env("CLAUDE_JOB_DIR", "/home/user/.claude/jobs/0b0b0b0b")
            .env("CLAUDE_CODE_ENTRYPOINT", "cli")
            .env("NGTERM_SESSION_ID", DAEMONS_SESSION)
            .env("NGTERM_PANE", "left")
            .env("NGTERM_PANE_ID", DAEMONS_PANE)
            .env("NGTERM_SOCKET", self.socket)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(mut input) = child.stdin.take() {
            let payload = claude_payload(event, id, &claude_transcript(self.root, id)?);
            let _ = input.write_all(payload.as_bytes());
        }
        Ok(outcome(&child.wait_with_output()?))
    }
}

/// The Claude Code of `session`'s main pane reports that it started the conversation `id`, to the instance at
/// `socket`.
fn started(root: &Path, socket: &Path, session: &str, id: &str) -> TestResult<(Option<i32>, String, String)> {
    let args = [
        "agent",
        "hook",
        "claude",
        "SessionStart",
        "--target",
        session,
        "--pane",
        "left",
        "--socket",
        socket.to_str().ok_or("a UTF-8 path")?,
    ];
    let payload = claude_payload("SessionStart", id, &claude_transcript(root, id)?);
    Ok(outcome(&hook(root, &args, &payload)?))
}

/// A background job whose conversation the daemon's instance does not know asks the default instance next. In a test
/// that is one under the test's own root, where a pane holds the conversation: a hook that resolved the default
/// socket from the developer's environment would reach their running ngterm and leave this pane idle.
#[test]
fn a_background_jobs_hooks_ask_the_default_instance_under_the_tests_own_root() -> TestResult {
    let Some(bash) = codex_bash() else {
        eprintln!("SKIPPED: no Git for Windows bash");
        return Ok(());
    };
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let _daemons = serve_core(&socket)?;
    let state = state_dir(dir.path());
    std::fs::create_dir_all(&state)?;
    let default_socket = state.join("ngterm.sock");
    let _default = serve_core(&default_socket)?;
    let session = session_id(&default_socket)?;
    let silent = (Some(0), String::new(), String::new());
    assert_eq!(started(dir.path(), &default_socket, &session, ELSEWHERE)?, silent);
    let background = BackgroundJob {
        bash: &bash,
        root: dir.path(),
        socket: &socket,
    };
    assert_eq!(
        background.hook(
            "ngterm-claude-status.sh",
            &["active", "--blink"],
            "PostToolUse",
            ELSEWHERE
        )?,
        silent
    );
    assert_eq!(status_of(&default_socket)?, "active");
    assert_eq!(status_of(&socket)?, Value::Null);
    Ok(())
}

/// End to end through the installed Claude hooks, run as a background job's ([`BackgroundJob`]). A conversation
/// Claude Code moved there from a pane (its old transcript ends with `continued-in`) keeps that pane's status and
/// record up to date; a job no pane started does nothing and prints nothing.
#[test]
fn a_background_jobs_hooks_reach_the_pane_the_conversation_came_from_or_stay_silent() -> TestResult {
    let Some(bash) = codex_bash() else {
        eprintln!("SKIPPED: no Git for Windows bash");
        return Ok(());
    };
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let _server = serve_core(&socket)?;
    let session = session_id(&socket)?;
    let transcript = |id: &str| claude_transcript(dir.path(), id);
    let background = BackgroundJob {
        bash: &bash,
        root: dir.path(),
        socket: &socket,
    };
    let job = |script: &str, args: &[&str], event: &str, id: &str| background.hook(script, args, event, id);
    let silent = (Some(0), String::new(), String::new());

    // the pane's own Claude Code reports its conversation, then moves it to the daemon
    assert_eq!(started(dir.path(), &socket, &session, MOVED_FROM)?, silent);
    std::fs::write(
        transcript(MOVED_FROM)?,
        format!(
            "{{\"type\":\"continued-in\",\"sessionId\":\"{MOVED_FROM}\",\"continuedInSessionId\":\"{MOVED_TO}\"}}\n"
        ),
    )?;
    let idle = status_of(&socket)?;

    // a job no pane started: nothing happens, and nothing reaches the agent
    for (script, args, event) in [
        ("ngterm-agent-mail.sh", &["claude"][..], "PostToolUse"),
        ("ngterm-claude-status.sh", &["active", "--blink"][..], "PostToolUse"),
        ("ngterm-agent-restore.sh", &["claude", "start"][..], "SessionStart"),
    ] {
        assert_eq!(job(script, args, event, NOBODYS)?, silent, "{script}");
    }
    assert_eq!(status_of(&socket)?, idle);
    let agent = agent_get(&socket, &session)?.map_err(|err| err.clone())?;
    assert_eq!(agent["conversationId"], MOVED_FROM);

    // the moved conversation: its hooks are the pane's
    assert_eq!(
        job(
            "ngterm-claude-status.sh",
            &["active", "--blink"],
            "PostToolUse",
            MOVED_TO
        )?,
        silent
    );
    assert_eq!(status_of(&socket)?, "active");
    let agent = agent_get(&socket, &session)?.map_err(|err| err.clone())?;
    assert_eq!(
        (&agent["conversationId"], &agent["transcriptPath"]),
        (&json!(MOVED_TO), &json!(transcript(MOVED_TO)?))
    );
    // this core's seeded session has no live pane to keep a mailbox for: the refusal names the pane the hook ran as
    let (_, _, refusal) = job("ngterm-agent-mail.sh", &["claude"], "PostToolUse", MOVED_TO)?;
    assert!(
        refusal.contains(&session) && !refusal.contains(DAEMONS_SESSION),
        "{refusal}"
    );
    assert_eq!(
        job("ngterm-claude-status.sh", &["completed"], "Stop", MOVED_TO)?,
        silent
    );
    assert_eq!(status_of(&socket)?, "completed");
    Ok(())
}
