//! The chat's event model and its NDJSON form: one [`ChatEvent`] per line, `type` naming the body, a field absent
//! when unset.
//!
//! Every event has an `id`, stable across lines, so a later event with the same id updates the item an earlier one
//! made (a tool's result completes its call), and a `cursor`, the byte offset of the transcript line it came from. An
//! event read from a subagent's transcript carries `parent`, the id of the task tool that ran it.

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// The longest transcript line the parsers read; a longer one becomes an `unknown` event of entry `oversized`.
pub const LINE_CAP: usize = 8 * 1024 * 1024;

/// The most bytes of a tool's output an event carries.
pub const OUTPUT_CAP: usize = 4 * 1024;

/// The most bytes of an edit's unified diff an event carries.
pub const DIFF_CAP: usize = 64 * 1024;

/// The most bytes of a tool's one-line title.
pub const TITLE_CAP: usize = 200;

/// The `unknown` entry of a line that is not a JSON object.
pub const MALFORMED: &str = "malformed";

/// The `unknown` entry of a line over [`LINE_CAP`].
pub const OVERSIZED: &str = "oversized";

/// One chat event: an NDJSON line of `ngtermctl agent chat`.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatEvent {
    /// Stable across lines: a later event with this id updates the same item.
    pub id: String,
    /// The byte offset of the transcript line this event came from.
    pub cursor: u64,
    /// The task tool's id, for an event from a subagent's transcript.
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(flatten)]
    pub body: EventBody,
}

/// Which agent wrote the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Claude,
    Codex,
}

impl AgentKind {
    /// The wire spelling: `claude` or `codex`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    /// Parses the wire spelling; `None` for anything else.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Claude, Self::Codex]
            .into_iter()
            .find(|agent| agent.name() == name)
    }
}

/// What an event says; `type` on the wire.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventBody {
    /// The conversation the transcript holds; repeated with the same id when its model becomes known or changes.
    Conversation {
        agent: AgentKind,
        conversation_id: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        model: Option<String>,
    },
    /// A prompt the user sent.
    User {
        text: String,
        /// How many images the prompt carried; absent for none.
        #[serde(default)]
        images: Option<u32>,
    },
    /// The agent's reply, Markdown.
    Assistant {
        text: String,
    },
    /// The agent thought; shown collapsed. Claude writes the text empty, Codex a summary title.
    Thinking {
        text: String,
    },
    Tool(Tool),
    /// Claude's `AskUserQuestion`.
    Question(Question),
    /// A permission prompt waiting for the user.
    Approval(Approval),
    /// The permission prompt was answered.
    ApprovalDone(Approval),
    Notice {
        kind: NoticeKind,
        text: String,
        /// More to show on demand, e.g. a compaction's summary.
        #[serde(default)]
        detail: Option<String>,
    },
    /// A line the parser does not know: its raw entry type, [`MALFORMED`] or [`OVERSIZED`].
    Unknown {
        entry: String,
    },
    /// The conversation switched: drop what was shown.
    Reset,
    /// The backlog is done and following has started.
    Live,
}

/// A tool call, and once its result is written, how it went.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    /// The agent's own tool name, e.g. `Bash`; empty for a result whose call is not in the window.
    pub name: String,
    pub kind: ToolKind,
    /// One line: the command, the path, the URL or the query.
    pub title: String,
    pub status: ToolStatus,
    /// The output's first [`OUTPUT_CAP`] bytes.
    #[serde(default)]
    pub output: Option<String>,
    /// An edit's unified diff, its first [`DIFF_CAP`] bytes.
    #[serde(default)]
    pub diff: Option<String>,
    /// `true` when `output` or `diff` was cut at its cap.
    #[serde(default)]
    pub truncated: Option<bool>,
}

/// What a tool does, for its icon and layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Shell,
    Edit,
    Read,
    Search,
    Web,
    Task,
    Other,
}

/// Where a tool call (or a question) stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    /// No result yet; for a question, not answered yet.
    Running,
    Ok,
    Error,
}

/// An `AskUserQuestion` call: its questions, and once answered, the answer the agent got.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub questions: Vec<QuestionItem>,
    pub status: ToolStatus,
    /// The result the agent got: the answers, or the refusal.
    #[serde(default)]
    pub output: Option<String>,
}

/// One question of an `AskUserQuestion` call.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionItem {
    pub question: String,
    /// The short tab label.
    #[serde(default)]
    pub header: Option<String>,
    /// Several options can be picked.
    pub multi_select: bool,
    pub options: Vec<QuestionOption>,
}

/// One option of a question.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// A permission prompt, from the agent's `PermissionRequest` hook payload.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub tool: String,
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub tool_use_id: Option<String>,
}

/// What a notice reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    /// The context was compacted; `detail` holds the summary when the transcript has it.
    Compaction,
    /// The user interrupted a turn or dismissed a tool call.
    Interrupt,
    /// The agent's API call failed.
    ApiError,
    /// A conversation summary line.
    Summary,
}

impl ChatEvent {
    /// The event as one NDJSON line, without the newline.
    #[must_use]
    pub fn to_ndjson(&self) -> String {
        // A `ChatEvent` holds only strings, numbers and enums: serializing it cannot fail.
        serde_json::to_string(self).unwrap_or_default()
    }
}
