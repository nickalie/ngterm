//! The follower on temp files, read through a byte-counting reader and timed by a fake clock.

use std::cell::{Cell, RefCell};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use super::support::tool;
use crate::agent_chat::event::{LINE_CAP, OVERSIZED};
use crate::agent_chat::follow::{BACKLOG, CONTEXT, Clock, DISCOVERY_INTERVAL, Files, POLL_INTERVAL};
use crate::agent_chat::{AgentKind, ChatEvent, EventBody, Follower, ToolStatus};

/// Files on disk, counting every byte read.
#[derive(Debug, Clone, Default)]
struct Counting {
    read: Rc<Cell<u64>>,
}

#[derive(Debug)]
struct CountingReader {
    file: File,
    read: Rc<Cell<u64>>,
}

impl Read for CountingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.file.read(buf)?;
        self.read.set(self.read.get() + read as u64);
        Ok(read)
    }
}

impl Seek for CountingReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

impl Files for Counting {
    type Reader = CountingReader;

    fn open(&self, path: &Path) -> io::Result<CountingReader> {
        Ok(CountingReader {
            file: File::open(path)?,
            read: self.read.clone(),
        })
    }

    fn size(&self, path: &Path) -> io::Result<u64> {
        Ok(fs::metadata(path)?.len())
    }
}

/// A clock that moves only when slept on.
#[derive(Debug, Clone)]
struct FakeClock {
    now: Rc<Cell<Instant>>,
    sleeps: Rc<RefCell<Vec<Duration>>>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            now: Rc::new(Cell::new(Instant::now())),
            sleeps: Rc::default(),
        }
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.now.get()
    }

    fn sleep(&self, duration: Duration) {
        self.sleeps.borrow_mut().push(duration);
        self.now.set(self.now.get() + duration);
    }
}

type Under = Follower<Counting, FakeClock>;

fn follower(agent: AgentKind, path: &Path) -> (Under, Counting, FakeClock) {
    let files = Counting::default();
    let clock = FakeClock::new();
    (Follower::with(files.clone(), clock.clone(), agent, path), files, clock)
}

fn claude(path: &Path) -> (Under, Counting, FakeClock) {
    follower(AgentKind::Claude, path)
}

/// A Claude `user` line (no newline) whose prompt is `text`; one `user` event.
fn user(text: &str) -> String {
    serde_json::json!({"type": "user", "uuid": text, "message": {"content": text}}).to_string()
}

fn prompt(index: usize) -> String {
    format!("prompt {index}")
}

/// A transcript of `count` prompts, and each line's offset (plus the file's length last).
fn prompts(path: &Path, count: usize) -> Vec<u64> {
    let mut out = BufWriter::new(File::create(path).unwrap());
    let mut cursors = vec![0];
    for index in 0..count {
        let line = user(&prompt(index)) + "\n";
        out.write_all(line.as_bytes()).unwrap();
        cursors.push(cursors.last().unwrap() + line.len() as u64);
    }
    out.flush().unwrap();
    cursors
}

fn append(path: &Path, bytes: &[u8]) {
    OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
}

/// The `user` events' texts.
fn texts(events: &[ChatEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::User { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn expected(range: std::ops::Range<usize>) -> Vec<String> {
    range.map(prompt).collect()
}

fn is_live(event: &ChatEvent, cursor: u64) -> bool {
    event.body == EventBody::Live && event.cursor == cursor
}

fn transcript(dir: &TempDir) -> PathBuf {
    dir.path().join("transcript.jsonl")
}

#[test]
fn the_backlog_is_the_last_events_read_backward() {
    let dir = TempDir::new().unwrap();
    let path = transcript(&dir);
    let cursors = prompts(&path, 5000);
    let (mut follower, files, _) = claude(&path);
    let events = follower.backlog(50).unwrap();
    assert_eq!(texts(&events), expected(4950..5000));
    assert_eq!(events.len(), 51, "the prompts, then live");
    assert!(is_live(&events[50], cursors[5000]), "following starts at the end");
    assert_eq!(events[0].cursor, cursors[4950]);
    assert!(files.read.get() < cursors[5000] / 2, "{} bytes read", files.read.get());
}

#[test]
fn a_result_in_the_window_keeps_its_call_from_before_it() {
    let dir = TempDir::new().unwrap();
    let path = transcript(&dir);
    let call = r#"{"type":"assistant","uuid":"a","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"echo hi"}}]}}"#;
    let result = r#"{"type":"user","uuid":"r","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"hi"}]}}"#;
    let mut lines = vec![call.to_owned()];
    lines.extend((0..20).map(|index| user(&prompt(index))));
    lines.push(result.to_owned());
    lines.extend((20..23).map(|index| user(&prompt(index))));
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    let events = claude(&path).0.backlog(4).unwrap();
    assert_eq!(events.len(), 5, "{events:#?}");
    let done = tool(&events[0]);
    assert_eq!(events[0].id, "t1");
    assert_eq!(
        (done.name.as_str(), done.title.as_str(), done.status),
        ("Bash", "echo hi", ToolStatus::Ok),
        "the call before the window names the result's tool"
    );
    assert_eq!(texts(&events), expected(20..23));
}

#[test]
fn a_page_ends_before_its_cursor() {
    let dir = TempDir::new().unwrap();
    let path = transcript(&dir);
    let cursors = prompts(&path, 5000);
    let (mut follower, ..) = claude(&path);
    follower.backlog(10).unwrap();
    let page = follower.page(cursors[950], 50).unwrap();
    assert_eq!(texts(&page), expected(900..950));
    assert_eq!(page.len(), 50, "no live: a page is not following");
    assert_eq!(texts(&follower.page(cursors[30], 50).unwrap()), expected(0..30));
    assert!(follower.page(0, 50).unwrap().is_empty(), "the start was reached");
    let mid_line = follower.page(cursors[950] + 3, 2).unwrap();
    assert_eq!(
        texts(&mid_line),
        expected(948..950),
        "a cursor inside a line ends the page before it"
    );
    assert!(
        follower.poll().unwrap().is_empty(),
        "paging leaves following where it was"
    );
}

#[test]
fn after_a_cursor_resends_its_line_then_follows() {
    let dir = TempDir::new().unwrap();
    let path = transcript(&dir);
    let cursors = prompts(&path, 5000);
    let (mut follower, files, _) = claude(&path);
    let events = follower.after(cursors[4990]).unwrap();
    assert_eq!(texts(&events), expected(4990..5000));
    assert!(is_live(events.last().unwrap(), cursors[5000]));
    assert!(files.read.get() < 3 * CONTEXT, "{} bytes read", files.read.get());
    let inside = claude(&path).0.after(cursors[4990] + 5).unwrap();
    assert_eq!(
        texts(&inside),
        expected(4990..5000),
        "a cursor inside a line starts at that line"
    );
    append(&path, format!("{}\n{}\n", user("new 1"), user("new 2")).as_bytes());
    assert_eq!(texts(&follower.poll().unwrap()), ["new 1", "new 2"]);
}

#[test]
fn after_a_cursor_past_the_end_resets() {
    let dir = TempDir::new().unwrap();
    let path = transcript(&dir);
    let cursors = prompts(&path, 300);
    let events = claude(&path).0.after(cursors[300] + 10).unwrap();
    assert_eq!(events[0].body, EventBody::Reset);
    assert_eq!(texts(&events), expected(100..300), "then the backlog of {BACKLOG}");
    assert!(is_live(events.last().unwrap(), cursors[300]));
}

#[test]
fn appended_lines_and_a_partial_line_completed_later() {
    let dir = TempDir::new().unwrap();
    let path = transcript(&dir);
    let cursors = prompts(&path, 3);
    let (mut follower, ..) = claude(&path);
    assert_eq!(texts(&follower.backlog(10).unwrap()), expected(0..3));
    assert!(follower.poll().unwrap().is_empty());
    let next = user("next");
    let (head, tail) = next.split_at(10);
    append(&path, format!("{}\n{head}", user("whole")).as_bytes());
    let whole = follower.poll().unwrap();
    assert_eq!(texts(&whole), ["whole"]);
    assert_eq!(whole[0].cursor, cursors[3]);
    assert!(
        follower.poll().unwrap().is_empty(),
        "a partial line waits for its newline"
    );
    append(&path, format!("{tail}\n").as_bytes());
    let completed = follower.poll().unwrap();
    assert_eq!(texts(&completed), ["next"]);
    assert_eq!(completed[0].cursor, cursors[3] + user("whole").len() as u64 + 1);
}

#[test]
fn a_truncated_transcript_resets() {
    let dir = TempDir::new().unwrap();
    let path = transcript(&dir);
    prompts(&path, 10);
    let (mut follower, ..) = claude(&path);
    follower.backlog(BACKLOG).unwrap();
    fs::write(&path, user("after") + "\n").unwrap();
    let events = follower.poll().unwrap();
    assert_eq!(events[0].body, EventBody::Reset);
    assert_eq!(texts(&events), ["after"]);
    assert!(is_live(events.last().unwrap(), user("after").len() as u64 + 1));
}

#[test]
fn a_replaced_transcript_resets() {
    let dir = TempDir::new().unwrap();
    let path = transcript(&dir);
    prompts(&path, 10);
    let (mut follower, ..) = claude(&path);
    follower.backlog(BACKLOG).unwrap();
    let other = dir.path().join("other.jsonl");
    let lines: Vec<String> = (0..30).map(|index| user(&format!("other {index}"))).collect();
    fs::write(&other, lines.join("\n") + "\n").unwrap();
    fs::rename(&other, &path).unwrap();
    let events = follower.poll().unwrap();
    assert_eq!(events[0].body, EventBody::Reset, "a longer file with other first bytes");
    assert_eq!(texts(&events).len(), 30);
    assert_eq!(events.last().unwrap().body, EventBody::Live);
}

#[test]
fn a_50_mb_transcript_costs_only_its_window() {
    let dir = TempDir::new().unwrap();
    let path = transcript(&dir);
    let mut out = BufWriter::new(File::create(&path).unwrap());
    let pad = "x".repeat(1000);
    let mut cursors = vec![0];
    while *cursors.last().unwrap() < 50 * 1024 * 1024 {
        let line = user(&format!("{} {pad}", cursors.len())) + "\n";
        out.write_all(line.as_bytes()).unwrap();
        cursors.push(cursors.last().unwrap() + line.len() as u64);
    }
    out.flush().unwrap();
    drop(out);
    let (mut follower, files, _) = claude(&path);
    let events = follower.backlog(BACKLOG).unwrap();
    assert_eq!(events.len(), BACKLOG + 1);
    let end = *cursors.last().unwrap();
    let window = end - events[0].cursor;
    // The last doubling step overshoots by at most what was read before it; the fingerprint is 1 KiB.
    let bound = 2 * (window + CONTEXT) + 64 * 1024 + 1024;
    let read = files.read.get();
    assert!(
        read <= bound,
        "{read} bytes read for a {window}-byte window (bound {bound})"
    );
    assert!(read < 1024 * 1024, "{read} bytes read");
}

fn oversized_line() -> Vec<u8> {
    let mut line = vec![b'a'; LINE_CAP + 1024 * 1024];
    line[0] = b'{';
    line
}

fn is_oversized(event: &ChatEvent, cursor: u64) -> bool {
    event.cursor == cursor
        && event.body
            == EventBody::Unknown {
                entry: OVERSIZED.into(),
            }
}

#[test]
fn a_line_over_the_cap_is_unknown_and_never_held_whole() {
    let dir = TempDir::new().unwrap();
    let path = transcript(&dir);
    let cursors = prompts(&path, 1);
    let (mut follower, ..) = claude(&path);
    follower.backlog(10).unwrap();
    let big = oversized_line();
    append(&path, &big);
    assert!(follower.poll().unwrap().is_empty(), "no newline yet");
    assert_eq!(follower.buffered(), 0, "the line's bytes were dropped at the cap");
    append(&path, format!("\n{}\n", user("after")).as_bytes());
    let events = follower.poll().unwrap();
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(is_oversized(&events[0], cursors[1]));
    assert_eq!(texts(&events), ["after"]);
    let backward = claude(&path).0.backlog(10).unwrap();
    assert_eq!(texts(&backward), ["prompt 0", "after"]);
    assert!(is_oversized(&backward[1], cursors[1]), "read backward too");
}

#[test]
fn a_missing_transcript_is_read_once_it_appears() {
    let dir = TempDir::new().unwrap();
    let path = transcript(&dir);
    let (mut follower, ..) = claude(&path);
    let events = follower.backlog(10).unwrap();
    assert_eq!(events.len(), 1);
    assert!(is_live(&events[0], 0));
    assert!(follower.poll().unwrap().is_empty());
    prompts(&path, 2);
    assert_eq!(texts(&follower.poll().unwrap()), expected(0..2));
}

#[test]
fn following_polls_on_the_interval() {
    let dir = TempDir::new().unwrap();
    let path = transcript(&dir);
    prompts(&path, 2);
    let (follower, _, clock) = claude(&path);
    let mut follower = follower.interval(Duration::from_millis(100));
    follower.backlog(10).unwrap();
    let mut polls = 0;
    let mut seen = Vec::new();
    follower
        .follow(|events| {
            polls += 1;
            seen.extend(events);
            if polls == 1 {
                append(&path, format!("{}\n", user("later")).as_bytes());
            }
            Ok(if polls == 3 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            })
        })
        .unwrap();
    assert_eq!(texts(&seen), ["later"]);
    assert_eq!(*clock.sleeps.borrow(), [Duration::from_millis(100); 2]);
    assert_eq!(POLL_INTERVAL, Duration::from_millis(250), "the default");
}

#[test]
fn a_codex_window_reads_the_rollouts_first_line() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("rollout.jsonl");
    let meta = r#"{"ordinal":0,"type":"session_meta","payload":{"id":"thread-1","cwd":"/home/user/project"}}"#;
    let filler = r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}"#;
    let context = r#"{"type":"turn_context","payload":{"turn_id":"t","model":"gpt-test"}}"#;
    let mut lines = vec![meta];
    lines.extend(std::iter::repeat_n(filler, 4000));
    lines.push(context);
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    let events = follower(AgentKind::Codex, &path).0.backlog(1).unwrap();
    assert_eq!(events.len(), 2, "{events:#?}");
    assert_eq!(
        events[0].body,
        EventBody::Conversation {
            agent: AgentKind::Codex,
            conversation_id: "thread-1".into(),
            cwd: Some("/home/user/project".into()),
            model: Some("gpt-test".into()),
        },
        "the session_meta far before the window still names the conversation"
    );
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agent_chat")
}

/// The golden NDJSON at `path`.
fn golden(path: &Path) -> Vec<ChatEvent> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else if !entry.file_name().to_string_lossy().ends_with(".golden.ndjson") {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

const CLAUDE_MAIN: &str = "f559ab9f-a5bd-4b2f-91e5-72590d662bf4";
const CLAUDE_SUBAGENT: &str = "f559ab9f-a5bd-4b2f-91e5-72590d662bf4/subagents/agent-a3ef4fec6a51ca74c";

fn split(events: &[ChatEvent]) -> (Vec<ChatEvent>, Vec<ChatEvent>) {
    events
        .iter()
        .filter(|event| event.body != EventBody::Live)
        .cloned()
        .partition(|event| event.parent.is_none())
}

#[test]
fn claude_subagents_are_followed_under_their_task() {
    let source = fixtures().join("claude/projects/-home-user-project");
    let dir = TempDir::new().unwrap();
    copy_tree(&source, dir.path());
    let path = dir.path().join(format!("{CLAUDE_MAIN}.jsonl"));
    let events = claude(&path).0.backlog(BACKLOG).unwrap();
    let (main, subagent) = split(&events);
    assert_eq!(main, golden(&source.join(format!("{CLAUDE_MAIN}.golden.ndjson"))));
    assert_eq!(
        subagent,
        golden(&source.join(format!("{CLAUDE_SUBAGENT}.golden.ndjson")))
    );
}

#[test]
fn a_claude_subagent_written_later_is_found_on_a_later_look() {
    let source = fixtures().join("claude/projects/-home-user-project");
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(format!("{CLAUDE_MAIN}.jsonl"));
    fs::copy(source.join(format!("{CLAUDE_MAIN}.jsonl")), &path).unwrap();
    let (mut follower, _, clock) = claude(&path);
    follower.backlog(BACKLOG).unwrap();
    copy_tree(&source.join(CLAUDE_MAIN), &dir.path().join(CLAUDE_MAIN));
    assert!(follower.poll().unwrap().is_empty(), "not looked for again yet");
    clock.sleep(DISCOVERY_INTERVAL);
    let events = follower.poll().unwrap();
    assert_eq!(events, golden(&source.join(format!("{CLAUDE_SUBAGENT}.golden.ndjson"))));
}

#[test]
fn a_subagent_whose_task_is_not_shown_is_followed_from_its_end() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("main.jsonl");
    prompts(&path, 300);
    let subagents = dir.path().join("main/subagents");
    fs::create_dir_all(&subagents).unwrap();
    fs::write(subagents.join("agent-x.meta.json"), r#"{"toolUseId":"toolu_x"}"#).unwrap();
    fs::write(subagents.join("agent-x.jsonl"), user("old") + "\n").unwrap();
    let (mut follower, ..) = claude(&path);
    let events = follower.backlog(10).unwrap();
    assert_eq!(
        texts(&events),
        expected(290..300),
        "the task is not among the lines read"
    );
    append(
        &subagents.join("agent-x.jsonl"),
        format!("{}\n", user("new")).as_bytes(),
    );
    let events = follower.poll().unwrap();
    assert_eq!(texts(&events), ["new"]);
    assert_eq!(events[0].parent.as_deref(), Some("toolu_x"));
}

#[test]
fn codex_subagent_rollouts_are_followed_under_their_task() {
    let source = fixtures().join("codex/sessions/2026/11/03");
    let dir = TempDir::new().unwrap();
    let day = dir.path().join("sessions/2026/11/03");
    copy_tree(&source, &day);
    let main = "rollout-2026-11-03T23-01-46-01a1a300-e2e3-76a1-abcf-961cbbb2eb3c";
    let subagent = "rollout-2026-11-03T23-02-28-01a1a301-8737-70e2-bc57-72fa1f1f57c7";
    let events = follower(AgentKind::Codex, &day.join(format!("{main}.jsonl")))
        .0
        .backlog(BACKLOG)
        .unwrap();
    let (main_events, subagent_events) = split(&events);
    assert_eq!(main_events, golden(&source.join(format!("{main}.golden.ndjson"))));
    assert_eq!(
        subagent_events,
        golden(&source.join(format!("{subagent}.golden.ndjson")))
    );
}
