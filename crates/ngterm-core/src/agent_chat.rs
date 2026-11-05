//! The agent chat view's transcript reading: an agent's on-disk conversation (Claude Code's `projects/…/*.jsonl`,
//! Codex's rollout) turned into [`ChatEvent`]s, the NDJSON `ngtermctl agent chat` prints for the phone.
//!
//! - [`event`]: the event model and its NDJSON form.
//! - [`claude`]: Claude Code's transcript lines.
//! - [`codex`]: Codex's rollout lines.
//! - [`follow`]: reading a transcript file for the chat: the backlog, older pages and following.
//! - [`hook`]: an agent's hook payload, read for the origin's per-pane agent record (`agent.report`).
//! - [`approvals`]: the permission prompts the chat interleaves with a transcript's events.
//! - [`catalog`]: the agent's slash commands and skills, for the composer's `/` dropdown.
//! - [`account`]: the account the agent runs under and its weekly limit, for the `account` event.
//!
//! A parser reads one line at a time through [`TranscriptParser`], keeping what pairs a later line with an earlier
//! one (a tool's result with its call). The formats are undocumented and drift between agent versions, so a line a
//! parser does not know becomes an `unknown` event rather than an error. The fixtures under
//! `tests/fixtures/agent_chat/` record the versions read and what was found about each format.

pub mod account;
pub mod approvals;
pub mod catalog;
pub mod claude;
pub mod codex;
pub mod event;
pub mod follow;
pub mod hook;

pub use claude::ClaudeParser;
pub use codex::CodexParser;
pub use event::{
    AgentKind, Approval, ChatEvent, EventBody, NoticeKind, Question, QuestionItem, QuestionOption, Tool, ToolKind,
    ToolStatus,
};
pub use follow::Follower;

/// `agent.get`'s refusal for a pane with no agent record; `agent chat` ends quietly on it once following.
pub const NO_AGENT: &str = "no agent in this pane";

use serde_json::Value;

use event::{LINE_CAP, MALFORMED, OVERSIZED};

/// Turns an agent's transcript lines into chat events.
pub trait TranscriptParser {
    /// The events one line makes. `line` is the line without its newline, `cursor` its byte offset in the file. A
    /// blank line makes none; a line that won't parse makes one `unknown` event.
    fn parse_line(&mut self, line: &[u8], cursor: u64) -> Vec<ChatEvent>;
}

/// A transcript line, decoded.
enum Line {
    /// Whitespace only.
    Blank,
    /// A JSON object.
    Entry(Value),
    /// An `unknown` event: the line is over [`LINE_CAP`] or not a JSON object.
    Unknown(ChatEvent),
}

/// Decodes a line as a JSON object. Bytes that are not UTF-8 become U+FFFD, so one bad byte in a tool's output costs
/// that byte, not the line.
fn decode_line(line: &[u8], cursor: u64) -> Line {
    if line.len() > LINE_CAP {
        return Line::Unknown(unknown(cursor, OVERSIZED));
    }
    let text = String::from_utf8_lossy(line);
    let text = text.trim();
    if text.is_empty() {
        return Line::Blank;
    }
    match serde_json::from_str::<Value>(text) {
        Ok(entry) if entry.is_object() => Line::Entry(entry),
        _ => Line::Unknown(unknown(cursor, MALFORMED)),
    }
}

/// The id of an event a line makes when the line names no id of its own: `line-<cursor>`, or `<source>:line-<cursor>`
/// for a subagent's file, whose offsets are its own while its events share one stream with the main transcript's.
fn line_id(source: Option<&str>, cursor: u64) -> String {
    match source {
        Some(source) => format!("{source}:line-{cursor}"),
        None => format!("line-{cursor}"),
    }
}

/// An `unknown` event for the line at `cursor`.
fn unknown(cursor: u64, entry: &str) -> ChatEvent {
    ChatEvent {
        id: line_id(None, cursor),
        cursor,
        parent: None,
        body: EventBody::Unknown {
            entry: entry.to_owned(),
        },
    }
}

/// `text` cut to at most `cap` bytes on a character boundary, and whether it was cut.
fn cap(text: &str, cap: usize) -> (String, bool) {
    if text.len() <= cap {
        return (text.to_owned(), false);
    }
    let end = text.floor_char_boundary(cap);
    (text[..end].to_owned(), true)
}

/// The first line of `text`, cut to [`event::TITLE_CAP`] bytes.
fn title_line(text: &str) -> String {
    cap(text.lines().next().unwrap_or_default().trim(), event::TITLE_CAP).0
}

/// The events one line makes.
struct Out {
    cursor: u64,
    parent: Option<String>,
    /// What names the line's file, for the ids [`Out::line_id`] makes.
    source: Option<String>,
    events: Vec<ChatEvent>,
}

impl Out {
    fn new(cursor: u64, parent: Option<String>, source: Option<String>) -> Self {
        Self {
            cursor,
            parent,
            source,
            events: Vec::new(),
        }
    }

    /// The id of an event the line makes when it names no id of its own.
    fn line_id(&self) -> String {
        line_id(self.source.as_deref(), self.cursor)
    }

    fn push(&mut self, id: impl Into<String>, body: EventBody) {
        self.events.push(ChatEvent {
            id: id.into(),
            cursor: self.cursor,
            parent: self.parent.clone(),
            body,
        });
    }
}

/// A tool whose call is not in the window: no name and no title.
fn untitled_tool(kind: ToolKind, status: ToolStatus) -> Tool {
    Tool {
        name: String::new(),
        kind,
        title: String::new(),
        status,
        output: None,
        diff: None,
        truncated: None,
    }
}

/// A call's result: its status and its output's first [`event::OUTPUT_CAP`] bytes.
fn finish(tool: &mut Tool, status: ToolStatus, output: &str) {
    tool.status = status;
    let (output, cut) = cap(output, event::OUTPUT_CAP);
    tool.output = (!output.is_empty()).then_some(output);
    if cut {
        tool.truncated = Some(true);
    }
}

fn notice(kind: NoticeKind, text: &str, detail: Option<String>) -> EventBody {
    EventBody::Notice {
        kind,
        text: text.to_owned(),
        detail,
    }
}

/// A compaction notice; `summary` is the compacted conversation's summary when the transcript has it.
fn compaction(summary: Option<String>) -> EventBody {
    notice(NoticeKind::Compaction, "Conversation compacted", summary)
}

/// The unified diff of `old` to `new` in file `path`.
fn unified(path: &str, old: &str, new: &str) -> String {
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .missing_newline_hint(false)
        .header(path, path)
        .to_string()
}

/// The text between `<name>` and `</name>`, trimmed.
fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let start = text.find(&format!("<{name}>"))? + name.len() + 2;
    let end = text[start..].find(&format!("</{name}>"))? + start;
    Some(text[start..end].trim())
}

fn str_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

/// The string at `pointer`, unless it is missing or empty.
fn non_empty<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    str_at(value, pointer).filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    mod account;
    mod approvals;
    mod catalog;
    mod claude;
    mod codex;
    mod follow;
    mod hook;
    mod support;
}
