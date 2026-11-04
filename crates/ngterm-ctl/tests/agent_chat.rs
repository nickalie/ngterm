//! `ngtermctl agent chat`, built, against an in-process control server over the fake main loop and a recorded Claude
//! transcript copied into a temp `projects/` tree: the backlog and following, `--before` and `--after`, a record that
//! moves to a new transcript or is cleared, the refusal with no record, stdin's end, and the permission prompts it
//! interleaves. The record comes from a core the test reports to (as the hooks do), or from a scripted `agent.get`
//! when the test moves it by hand. Every run's environment is the test's own (`support::isolated`).

mod support;

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ngterm_host::fakes::FakeMainLoop;
use ngterm_proto::Response;
use serde_json::{Value, json};
use support::{
    NGTERMCTL, Server, TestResult, fixtures, isolated, request, serve_core, serve_on, session_id, short_dir,
};

/// The recorded conversation with an approval allowed, one denied and one dismissed (`tests/fixtures/agent_chat`).
const MAIN: &str = "f559ab9f-a5bd-4b2f-91e5-72590d662bf4";
/// Two short conversations, for a record that moves.
const SHORT: &str = "17d240d5-8dc2-4980-9a49-0c5015321537";
const OTHER: &str = "2897645c-5554-4c74-ab67-20ec0af2f264";

/// Line starts in `MAIN`: the denied call's result (`touch denied.txt`), the next prompt, and the call after it.
const DENIED_RESULT: u64 = 72_526;
const NEXT_PROMPT: u64 = 75_878;
const NEXT_CALL: u64 = 76_498;

/// The longest a test waits for one line or an exit.
const WAIT: Duration = Duration::from_secs(10);

fn recorded(id: &str) -> PathBuf {
    fixtures().join("claude/projects/-home-user-project").join(id)
}

/// The recorded transcript `id`.
fn transcript(id: &str) -> TestResult<Vec<u8>> {
    Ok(fs::read(recorded(id).with_extension("jsonl"))?)
}

/// The events the Claude parser makes of transcript `id` (its golden file).
fn golden(id: &str) -> TestResult<Vec<Value>> {
    let text = fs::read_to_string(recorded(id).with_extension("golden.ndjson"))?;
    Ok(text.lines().map(serde_json::from_str).collect::<Result<_, _>>()?)
}

/// Where the test's Claude config keeps transcript `id`: a `projects/` tree, as the record's check wants.
fn transcript_path(root: &Path, id: &str) -> PathBuf {
    root.join("claude/projects/-home-user-project")
        .join(format!("{id}.jsonl"))
}

/// Writes `bytes` as transcript `id`, returning its path.
fn write_transcript(root: &Path, id: &str, bytes: &[u8]) -> TestResult<PathBuf> {
    let path = transcript_path(root, id);
    fs::create_dir_all(path.parent().ok_or("a parent")?)?;
    fs::write(&path, bytes)?;
    Ok(path)
}

fn append(path: &Path, bytes: &[u8]) -> TestResult {
    OpenOptions::new().append(true).open(path)?.write_all(bytes)?;
    Ok(())
}

fn slice(bytes: &[u8], from: u64, to: u64) -> TestResult<&[u8]> {
    let (from, to) = (usize::try_from(from)?, usize::try_from(to)?);
    Ok(&bytes[from..to.min(bytes.len())])
}

fn path_arg(path: &Path) -> TestResult<&str> {
    Ok(path.to_str().ok_or("a UTF-8 path")?)
}

/// A server whose `agent.get` answers with the record the test holds, or refuses as a core does without one.
struct Scripted {
    _server: Server<()>,
    record: Arc<Mutex<Option<Value>>>,
    gets: Arc<Mutex<Vec<Value>>>,
}

impl Scripted {
    fn serve(socket: &Path, record: Option<Value>) -> TestResult<Self> {
        let record = Arc::new(Mutex::new(record));
        let gets = Arc::new(Mutex::new(Vec::new()));
        let (held, seen) = (Arc::clone(&record), Arc::clone(&gets));
        let server = serve_on(socket, FakeMainLoop::spawn(())?, move |(), request| {
            if let Ok(mut seen) = seen.lock() {
                seen.push(serde_json::to_value(&request).unwrap_or_default());
            }
            match held.lock().ok().and_then(|record| record.clone()) {
                Some(agent) => Response::with_result(json!({"agent": agent})).into(),
                None => Response::failure("no agent in this pane").into(),
            }
        })?;
        Ok(Self {
            _server: server,
            record,
            gets,
        })
    }

    fn set(&self, record: Option<Value>) -> TestResult {
        *self.record.lock().map_err(|_| "poisoned")? = record;
        Ok(())
    }

    fn first_get(&self) -> TestResult<Value> {
        Ok(self
            .gets
            .lock()
            .map_err(|_| "poisoned")?
            .first()
            .cloned()
            .ok_or("no request")?)
    }
}

/// `agent.get`'s record of a Claude conversation in transcript `path`.
fn record(path: &Path, id: &str, approval: Option<Value>) -> TestResult<Value> {
    let mut record = json!({
        "agent": "claude",
        "pane": "left",
        "transcriptPath": path_arg(path)?,
        "conversationId": id,
        "updatedAt": 1.0,
    });
    if let Some(approval) = approval {
        record["approval"] = approval;
    }
    Ok(record)
}

fn denied_txt(since: f64) -> Value {
    json!({"tool": "Bash", "title": "Create empty file denied.txt", "detail": "touch denied.txt", "since": since})
}

/// A running `ngtermctl agent chat`: its stdout lines arrive on a channel. Dropping it kills the process.
struct Chat {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl Chat {
    fn spawn(root: &Path, socket: &Path, args: &[&str]) -> TestResult<Self> {
        let mut child = isolated(NGTERMCTL, root)
            .args(["agent", "chat", "--socket", path_arg(socket)?])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            stdin: child.stdin.take(),
            child,
            lines,
        })
    }

    /// The next event.
    fn next(&self) -> TestResult<Value> {
        let line = self
            .lines
            .recv_timeout(WAIT)
            .map_err(|error| format!("no event: {error}"))?;
        Ok(serde_json::from_str(&line)?)
    }

    /// The next `count` events.
    fn take(&self, count: usize) -> TestResult<Vec<Value>> {
        (0..count).map(|_| self.next()).collect()
    }

    /// The events up to the first of type `kind`, which is the last.
    fn until(&self, kind: &str) -> TestResult<Vec<Value>> {
        let mut events = Vec::new();
        loop {
            let event = self.next()?;
            let done = event["type"] == kind;
            events.push(event);
            if done {
                return Ok(events);
            }
        }
    }

    /// Whether no line comes for `quiet`.
    fn silent_for(&self, quiet: Duration) -> bool {
        self.lines.recv_timeout(quiet).is_err()
    }

    fn close_stdin(&mut self) {
        self.stdin = None;
    }

    /// The process's exit, once it comes within `limit`.
    fn exit_within(&mut self, limit: Duration) -> TestResult<ExitStatus> {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            if start.elapsed() > limit {
                return Err(format!("still running after {limit:?}").into());
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Chat {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The events of `golden` whose line starts in `from..to`.
fn lines_in(golden: &[Value], from: u64, to: u64) -> Vec<Value> {
    golden
        .iter()
        .filter(|event| {
            event["cursor"]
                .as_u64()
                .is_some_and(|cursor| (from..to).contains(&cursor))
        })
        .cloned()
        .collect()
}

/// Against a core the hooks' reports fill: the backlog, then `live` at the end of what was read, then each appended
/// line's events as they are written, as the parser makes them of the whole file; `SessionEnd` clearing the record
/// ends the stream with a zero exit.
#[test]
fn the_backlog_then_live_then_appended_lines_until_the_record_is_cleared() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let _core = serve_core(&socket)?;
    let session = session_id(&socket)?;
    let bytes = transcript(MAIN)?;
    let path = write_transcript(dir.path(), MAIN, slice(&bytes, 0, NEXT_PROMPT)?)?;
    let report = |event: &str| {
        json!({"cmd": "agent.report", "target": session, "args": {"pane": "left", "report": {
            "agent": "claude", "event": event, "transcriptPath": path_arg(&path).unwrap_or_default(),
            "conversationId": MAIN}}})
    };
    assert!(request(&socket, &report("SessionStart"))?.ok);

    let chat = Chat::spawn(dir.path(), &socket, &["--target", &session, "--pane", "left"])?;
    let golden = golden(MAIN)?;
    let mut backlog = chat.until("live")?;
    let live = backlog.pop().ok_or("no live")?;
    assert_eq!(live, json!({"id": "live", "cursor": NEXT_PROMPT, "type": "live"}));
    assert_eq!(backlog, lines_in(&golden, 0, NEXT_PROMPT));

    append(&path, slice(&bytes, NEXT_PROMPT, u64::MAX)?)?;
    let rest = lines_in(&golden, NEXT_PROMPT, u64::MAX);
    assert_eq!(chat.take(rest.len())?, rest);

    assert!(request(&socket, &report("SessionEnd"))?.ok);
    let mut chat = chat;
    assert!(
        chat.exit_within(WAIT)?.success(),
        "a cleared record ends the stream quietly"
    );
    Ok(())
}

/// No record: the refusal, and a non-zero exit, before anything is printed.
#[test]
fn with_no_record_it_fails_with_no_agent_in_this_pane() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let _core = serve_core(&socket)?;
    let session = session_id(&socket)?;
    let output = isolated(NGTERMCTL, dir.path())
        .args(["agent", "chat", "--target", &session, "--socket", path_arg(&socket)?])
        .stdin(Stdio::piped())
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no agent in this pane"), "{stderr}");
    Ok(())
}

/// `--before` prints one older page of whole lines, as the parser makes them, and exits without reading stdin; a page
/// before the start is empty.
#[test]
fn before_prints_one_older_page_and_exits() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let path = write_transcript(dir.path(), MAIN, &transcript(MAIN)?)?;
    let server = Scripted::serve(&socket, Some(record(&path, MAIN, None)?))?;
    let before = NEXT_PROMPT.to_string();
    let mut chat = Chat::spawn(
        dir.path(),
        &socket,
        &["--target", "S", "--pane", "right", "--before", &before, "--limit", "5"],
    )?;
    assert!(chat.exit_within(WAIT)?.success(), "a page exits with stdin still open");
    let page: Vec<Value> = chat
        .lines
        .try_iter()
        .map(|line| serde_json::from_str(&line))
        .collect::<Result<_, _>>()?;
    assert!(page.len() >= 5, "{page:#?}");
    let older = lines_in(&golden(MAIN)?, 0, NEXT_PROMPT);
    assert_eq!(page, older[older.len() - page.len()..]);
    assert_eq!(
        server.first_get()?,
        json!({"cmd": "agent.get", "target": "S", "args": {"pane": "right"}})
    );

    let mut chat = Chat::spawn(dir.path(), &socket, &["--target", "S", "--before", "0"])?;
    assert!(chat.exit_within(WAIT)?.success());
    assert_eq!(chat.lines.try_iter().count(), 0, "nothing before the start");
    Ok(())
}

/// `--after` sends the cursor's line again and everything after it, led by the conversation it belongs to, then
/// `live` at the end, and follows.
#[test]
fn after_resends_the_cursors_line_and_follows() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let bytes = transcript(MAIN)?;
    let path = write_transcript(dir.path(), MAIN, slice(&bytes, 0, NEXT_CALL)?)?;
    let _server = Scripted::serve(&socket, Some(record(&path, MAIN, None)?))?;
    let after = DENIED_RESULT.to_string();
    let chat = Chat::spawn(dir.path(), &socket, &["--target", "S", "--after", &after])?;
    let mut events = chat.until("live")?;
    assert_eq!(
        events.pop(),
        Some(json!({"id": "live", "cursor": NEXT_CALL, "type": "live"}))
    );
    let conversation = events.remove(0);
    assert_eq!(
        (
            &conversation["type"],
            &conversation["conversation_id"],
            &conversation["cursor"]
        ),
        (&json!("conversation"), &json!(MAIN), &json!(DENIED_RESULT))
    );
    let golden = golden(MAIN)?;
    assert_eq!(events, lines_in(&golden, DENIED_RESULT, NEXT_CALL));

    append(&path, slice(&bytes, NEXT_CALL, u64::MAX)?)?;
    let rest = lines_in(&golden, NEXT_CALL, u64::MAX);
    assert_eq!(chat.take(rest.len())?, rest);
    Ok(())
}

/// The record naming another transcript (a `/clear`, a resume): `reset`, then the new file's backlog and `live`.
#[test]
fn a_new_record_path_resets_to_the_new_transcript() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let short = write_transcript(dir.path(), SHORT, &transcript(SHORT)?)?;
    let other = write_transcript(dir.path(), OTHER, &transcript(OTHER)?)?;
    let server = Scripted::serve(&socket, Some(record(&short, SHORT, None)?))?;
    let chat = Chat::spawn(dir.path(), &socket, &["--target", "S"])?;
    let mut backlog = chat.until("live")?;
    backlog.pop();
    assert_eq!(backlog, golden(SHORT)?);

    server.set(Some(record(&other, OTHER, None)?))?;
    assert_eq!(chat.next()?, json!({"id": "reset", "cursor": 0, "type": "reset"}));
    let mut backlog = chat.until("live")?;
    backlog.pop();
    assert_eq!(backlog, golden(OTHER)?);
    Ok(())
}

/// The phone's ssh channel going away closes stdin: the process ends soon after, with a zero exit.
#[test]
fn stdin_closing_ends_the_process_within_a_bound() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let path = write_transcript(dir.path(), SHORT, &transcript(SHORT)?)?;
    let _server = Scripted::serve(&socket, Some(record(&path, SHORT, None)?))?;
    let mut chat = Chat::spawn(dir.path(), &socket, &["--target", "S"])?;
    chat.until("live")?;
    chat.close_stdin();
    assert!(chat.exit_within(Duration::from_secs(3))?.success());
    Ok(())
}

/// A live approval shows as `approval` after the backlog; the transcript writing the denied call's result ends it at
/// once, right after that result, and the record still holding it (the status has not left `blocked`) shows it no
/// more. A new candidate shows, and the record dropping it ends it.
#[test]
fn approvals_are_interleaved_and_end_by_the_transcript_or_the_record() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let bytes = transcript(MAIN)?;
    let path = write_transcript(dir.path(), MAIN, slice(&bytes, 0, DENIED_RESULT)?)?;
    let server = Scripted::serve(&socket, Some(record(&path, MAIN, Some(denied_txt(100.0)))?))?;
    let chat = Chat::spawn(dir.path(), &socket, &["--target", "S"])?;
    chat.until("live")?;
    let approval = json!({"tool": "Bash", "title": "Create empty file denied.txt", "detail": "touch denied.txt"});
    let mut shown = json!({"id": "approval-100", "cursor": DENIED_RESULT, "type": "approval"});
    merge(&mut shown, &approval);
    assert_eq!(chat.next()?, shown);

    append(&path, slice(&bytes, DENIED_RESULT, NEXT_PROMPT)?)?;
    let golden = golden(MAIN)?;
    let answered = lines_in(&golden, DENIED_RESULT, NEXT_PROMPT);
    let mut done = json!({"id": "approval-100", "cursor": DENIED_RESULT, "type": "approval_done"});
    merge(&mut done, &approval);
    assert_eq!(chat.next()?, answered[0], "the denied call's result");
    assert_eq!(chat.next()?, done, "ends the prompt right after it");
    assert_eq!(chat.take(answered.len() - 1)?, answered[1..]);

    // the record keeps the answered candidate for a while: it never shows again
    assert!(chat.silent_for(Duration::from_millis(1500)));
    append(&path, slice(&bytes, NEXT_PROMPT, NEXT_CALL)?)?;
    let prompt = lines_in(&golden, NEXT_PROMPT, NEXT_CALL);
    assert_eq!(chat.take(prompt.len())?, prompt);

    let interrupted = json!({"tool": "Bash", "title": "Create empty file interrupted.txt", "detail": "touch interrupted.txt", "since": 200.5});
    server.set(Some(record(&path, MAIN, Some(interrupted))?))?;
    let next = chat.next()?;
    assert_eq!(
        (&next["type"], &next["id"], &next["cursor"]),
        (&json!("approval"), &json!("approval-200.5"), &json!(NEXT_PROMPT))
    );
    server.set(Some(record(&path, MAIN, None)?))?;
    let next = chat.next()?;
    assert_eq!(
        (&next["type"], &next["id"]),
        (&json!("approval_done"), &json!("approval-200.5"))
    );
    Ok(())
}

/// The transcript answering a prompt before the record is looked at: the denial already in the backlog (a connect,
/// or a reconnect with `--after`) or read by a poll before the record shows the candidate. The record still holds
/// the candidate (the status has not left `blocked`), and it is never shown.
#[test]
fn a_prompt_the_transcript_answered_before_the_record_was_read_is_never_shown() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let bytes = transcript(MAIN)?;
    let path = write_transcript(dir.path(), MAIN, slice(&bytes, 0, NEXT_PROMPT)?)?;
    let server = Scripted::serve(&socket, Some(record(&path, MAIN, Some(denied_txt(100.0)))?))?;
    for args in [&["--target", "S"][..], &["--target", "S", "--after", "0"]] {
        let chat = Chat::spawn(dir.path(), &socket, args)?;
        chat.until("live")?;
        assert!(chat.silent_for(Duration::from_millis(1500)), "{args:?}");
    }

    fs::write(&path, slice(&bytes, 0, DENIED_RESULT)?)?;
    server.set(Some(record(&path, MAIN, None)?))?;
    let chat = Chat::spawn(dir.path(), &socket, &["--target", "S"])?;
    chat.until("live")?;
    append(&path, slice(&bytes, DENIED_RESULT, NEXT_PROMPT)?)?;
    let answered = lines_in(&golden(MAIN)?, DENIED_RESULT, NEXT_PROMPT);
    assert_eq!(chat.take(answered.len())?, answered);
    server.set(Some(record(&path, MAIN, Some(denied_txt(100.0)))?))?;
    assert!(chat.silent_for(Duration::from_millis(1500)));
    Ok(())
}

fn merge(into: &mut Value, from: &Value) {
    if let (Some(into), Some(from)) = (into.as_object_mut(), from.as_object()) {
        into.extend(from.iter().map(|(key, value)| (key.clone(), value.clone())));
    }
}
