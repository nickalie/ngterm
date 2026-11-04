//! `ngtermctl agent commands`, built, against an in-process control server whose `agent.get` answers with a record the
//! test holds: the catalog of a Claude conversation (its config dir and project from the transcript) and of a Codex
//! one, the refusal with no record, and a record whose transcript lies outside the agent's tree. Every run's
//! environment is the test's own (`support::isolated`), its config trees temp dirs.

#[expect(dead_code, reason = "the helpers the other agent tests use and this one does not")]
mod support;

use std::fs;
use std::path::Path;
use std::process::Output;
use std::sync::{Arc, Mutex};

use ngterm_host::fakes::FakeMainLoop;
use ngterm_proto::Response;
use serde_json::{Value, json};
use support::{NGTERMCTL, Server, TestResult, isolated, serve_on, short_dir};

fn write(path: &Path, text: &str) -> TestResult {
    fs::create_dir_all(path.parent().ok_or("a parent")?)?;
    fs::write(path, text)?;
    Ok(())
}

fn path_arg(path: &Path) -> TestResult<&str> {
    Ok(path.to_str().ok_or("a UTF-8 path")?)
}

/// The requests a server got.
type Seen = Arc<Mutex<Vec<Value>>>;

/// A server answering `agent.get` with `record` (or refusing as a core does without one), keeping the requests.
fn serve(socket: &Path, record: Option<Value>) -> TestResult<(Server<()>, Seen)> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&seen);
    let server = serve_on(socket, FakeMainLoop::spawn(())?, move |(), request| {
        if let Ok(mut kept) = kept.lock() {
            kept.push(serde_json::to_value(&request).unwrap_or_default());
        }
        match &record {
            Some(agent) => Response::with_result(json!({"agent": agent})).into(),
            None => Response::failure("no agent in this pane").into(),
        }
    })?;
    Ok((server, seen))
}

fn record(agent: &str, transcript: &Path) -> TestResult<Value> {
    Ok(json!({
        "agent": agent,
        "pane": "right",
        "transcriptPath": path_arg(transcript)?,
        "conversationId": "c1",
        "updatedAt": 1.0,
    }))
}

fn run(root: &Path, socket: &Path, extra: &[&str]) -> TestResult<Output> {
    Ok(isolated(NGTERMCTL, root)
        .args(["agent", "commands", "--socket", path_arg(socket)?])
        .args(extra)
        .output()?)
}

/// The catalog a successful run printed: one JSON array on one line.
fn printed(output: &Output) -> TestResult<Vec<Value>> {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout.clone())?;
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    Ok(serde_json::from_str(&stdout)?)
}

fn find<'a>(catalog: &'a [Value], name: &str) -> Option<&'a Value> {
    catalog.iter().find(|entry| entry["name"] == name)
}

#[test]
fn a_claude_record_lists_the_builtins_and_the_skills_of_its_config_and_project() -> TestResult {
    let dir = short_dir()?;
    let config = dir.path().join("my-claude");
    let project = dir.path().join("project");
    let transcript = config.join("projects/-project/c1.jsonl");
    let line = json!({"type": "user", "sessionId": "c1", "cwd": project, "message": {"role": "user", "content": "hi"}});
    write(&transcript, &format!("{line}\n"))?;
    write(
        &config.join("skills/deploy/SKILL.md"),
        "---\nname: deploy\ndescription: Ship it\nargument-hint: \"[env]\"\n---\n",
    )?;
    write(&project.join(".claude/commands/team/sync.md"), "Sync the team.\n")?;
    let socket = dir.path().join("s.sock");
    let (_server, seen) = serve(&socket, Some(record("claude", &transcript)?))?;

    let catalog = printed(&run(dir.path(), &socket, &["--target", "S1", "--pane", "right"])?)?;
    assert_eq!(
        find(&catalog, "deploy"),
        Some(&json!({"name": "deploy", "description": "Ship it", "kind": "skill", "argument_hint": "[env]"}))
    );
    assert_eq!(
        find(&catalog, "team:sync"),
        Some(&json!({"name": "team:sync", "description": "Sync the team.", "kind": "command"}))
    );
    assert_eq!(
        find(&catalog, "compact").map(|entry| &entry["kind"]),
        Some(&json!("builtin"))
    );
    let requests = seen.lock().map_err(|_| "poisoned")?.clone();
    assert_eq!(
        requests,
        [json!({"cmd": "agent.get", "target": "S1", "args": {"pane": "right"}})]
    );
    Ok(())
}

#[test]
fn a_codex_record_lists_its_skills_as_mentions() -> TestResult {
    let dir = short_dir()?;
    let home = dir.path().join("codex-home");
    let rollout = home.join("sessions/2026/11/03/rollout-c1.jsonl");
    write(
        &rollout,
        &format!("{}\n", json!({"type": "session_meta", "payload": {"id": "c1"}})),
    )?;
    write(
        &home.join("skills/greet-check/SKILL.md"),
        "---\nname: greet-check\ndescription: Check the greeting\n---\n",
    )?;
    let socket = dir.path().join("s.sock");
    let (_server, _) = serve(&socket, Some(record("codex", &rollout)?))?;

    let catalog = printed(&run(dir.path(), &socket, &[])?)?;
    assert_eq!(
        find(&catalog, "$greet-check"),
        Some(&json!({"name": "$greet-check", "description": "Check the greeting", "kind": "skill"}))
    );
    assert_eq!(
        find(&catalog, "review").map(|entry| &entry["kind"]),
        Some(&json!("builtin"))
    );
    Ok(())
}

#[test]
fn with_no_record_it_fails_with_no_agent_in_this_pane() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("s.sock");
    let (_server, _) = serve(&socket, None)?;
    let output = run(dir.path(), &socket, &[])?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no agent in this pane"), "{stderr}");
    Ok(())
}

#[test]
fn a_transcript_outside_the_agents_tree_is_refused() -> TestResult {
    let dir = short_dir()?;
    let stray = dir.path().join("stray.jsonl");
    write(&stray, "")?;
    let socket = dir.path().join("s.sock");
    let (_server, _) = serve(&socket, Some(record("claude", &stray)?))?;
    let output = run(dir.path(), &socket, &[])?;
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("transcript path is outside a Claude projects/ tree"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn a_scratch_or_unknown_pane_is_a_usage_error() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("s.sock");
    for pane in ["scratch", "middle"] {
        let output = run(dir.path(), &socket, &["--pane", pane])?;
        assert_eq!(output.status.code(), Some(64), "--pane {pane}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("--pane must be left or right"), "{stderr}");
    }
    Ok(())
}
