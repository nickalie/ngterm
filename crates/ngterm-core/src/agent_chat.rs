//! The agent chat view's transcript reading: an agent's on-disk conversation (Claude Code's `projects/…/*.jsonl`,
//! Codex's rollout) turned into [`ChatEvent`]s, the NDJSON `ngtermctl agent chat` prints for the phone.
//!
//! - [`event`]: the event model and its NDJSON form.
//! - [`claude`]: Claude Code's transcript lines.
//!
//! A parser reads one line at a time through [`TranscriptParser`], keeping what pairs a later line with an earlier
//! one (a tool's result with its call). The formats are undocumented and drift between agent versions, so a line a
//! parser does not know becomes an `unknown` event rather than an error. The fixtures under
//! `tests/fixtures/agent_chat/` record the versions read and what was found about each format.

pub mod claude;
pub mod event;

pub use claude::ClaudeParser;
pub use event::{
    AgentKind, Approval, ChatEvent, EventBody, NoticeKind, Question, QuestionItem, QuestionOption, Tool, ToolKind,
    ToolStatus,
};

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

/// The id of an event a line makes when the line names no id of its own.
fn line_id(cursor: u64) -> String {
    format!("line-{cursor}")
}

/// An `unknown` event for the line at `cursor`.
fn unknown(cursor: u64, entry: &str) -> ChatEvent {
    ChatEvent {
        id: line_id(cursor),
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

#[cfg(test)]
mod tests {
    mod claude;
}
