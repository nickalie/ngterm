//! What a pane's agent chat tells the app, as `UniFFI` records: the origin's chat events
//! ([`ngterm_core::agent_chat::ChatEvent`], the NDJSON lines of `ngtermctl agent chat`) and the slash-command catalog
//! ([`ngterm_core::agent_chat::catalog::SlashCommand`]) flattened for Kotlin. An agent travels as its wire name
//! (`claude`, `codex`), as the presentation stream's agent frames name it.

use ngterm_core::agent_chat::catalog::{CommandKind, SlashCommand};
use ngterm_core::agent_chat::{
    self as wire, Approval, EventBody, NoticeKind, Question, QuestionItem, QuestionOption, Tool, ToolKind, ToolStatus,
};

/// One event of a pane's chat. A later event with the same `id` updates the item an earlier one made (a tool's result
/// completes its call); the app upserts by it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChatEvent {
    pub id: String,
    /// The byte offset of the transcript line the event came from (a subagent's, in its own file).
    pub cursor: u64,
    /// The task tool's id, for an event from a subagent's transcript: it folds under that task.
    pub parent: Option<String>,
    pub body: ChatBody,
}

/// What a chat event says.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ChatBody {
    /// The conversation the transcript holds; repeated with the same id when its model becomes known or changes.
    Conversation {
        agent: String,
        conversation_id: String,
        cwd: Option<String>,
        model: Option<String>,
    },
    /// A prompt the user sent, with how many images it carried.
    User {
        text: String,
        images: u32,
    },
    /// The agent's reply, Markdown.
    Assistant {
        text: String,
    },
    /// The agent thought; shown collapsed. Claude writes the text empty, Codex a summary title.
    Thinking {
        text: String,
    },
    Tool {
        tool: ChatTool,
    },
    /// Claude's `AskUserQuestion`.
    Question {
        question: ChatQuestion,
    },
    /// A permission prompt waiting for the user.
    Approval {
        approval: ChatApproval,
    },
    /// The permission prompt was answered (here or anywhere else).
    ApprovalDone {
        approval: ChatApproval,
    },
    Notice {
        kind: ChatNoticeKind,
        text: String,
        /// More to show on demand, e.g. a compaction's summary.
        detail: Option<String>,
    },
    /// A transcript line the origin does not know: its raw entry type, `malformed` or `oversized`.
    Unknown {
        entry: String,
    },
    /// The conversation switched: drop what was shown.
    Reset,
    /// The backlog (or a reconnect's catch-up) is done and following has started.
    Live,
}

/// A tool call, and once its result is written, how it went.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChatTool {
    /// The agent's own tool name, e.g. `Bash`; empty for a result whose call is not in the window.
    pub name: String,
    pub kind: ChatToolKind,
    /// One line: the command, the path, the URL or the query.
    pub title: String,
    pub status: ChatToolStatus,
    /// The output's first 4 KiB.
    pub output: Option<String>,
    /// An edit's unified diff, its first 64 KiB.
    pub diff: Option<String>,
    /// Whether the output or the diff was cut at its cap.
    pub truncated: bool,
}

/// What a tool does, for its glyph and layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ChatToolKind {
    Shell,
    Edit,
    Read,
    Search,
    Web,
    Task,
    Other,
}

/// Where a tool call or a question stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ChatToolStatus {
    /// No result yet; for a question, not answered yet.
    Running,
    Ok,
    Error,
}

/// An `AskUserQuestion` call: its questions, and once answered, the answer the agent got.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChatQuestion {
    pub questions: Vec<ChatQuestionItem>,
    pub status: ChatToolStatus,
    /// The result the agent got: the answers, or the refusal.
    pub output: Option<String>,
}

/// One question of an `AskUserQuestion` call.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChatQuestionItem {
    pub question: String,
    /// The short tab label.
    pub header: Option<String>,
    /// Several options can be picked.
    pub multi_select: bool,
    pub options: Vec<ChatQuestionOption>,
}

/// One option of a question.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChatQuestionOption {
    pub label: String,
    pub description: Option<String>,
}

/// A permission prompt, from the agent's hook payload.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChatApproval {
    pub tool: String,
    pub title: String,
    pub detail: Option<String>,
    pub tool_use_id: Option<String>,
    /// The agent's working directory when it asked: what its dialog shows a file's path relative to.
    pub cwd: Option<String>,
}

/// What a notice reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ChatNoticeKind {
    /// The context was compacted; the detail holds the summary when the transcript has it.
    Compaction,
    /// The user interrupted a turn or dismissed a tool call.
    Interrupt,
    /// The agent's API call failed.
    ApiError,
    /// A conversation summary line.
    Summary,
}

/// What became of a pane's open chat, besides its events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ChatState {
    /// Its channel went (the connection was lost, the origin's command failed): it resumes on its own from where it
    /// stood, and the next [`ChatBody::Live`] says it caught up.
    Reconnecting,
    /// It ended for good: the agent ended, the pane runs none, or the origin offers no chat. Show the terminal; the
    /// chat is closed, and opening it again is the app's call.
    Ended,
}

/// One entry of the composer's `/` dropdown: an agent's slash command or skill.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentCommand {
    /// What follows the `/`; a Codex skill's `$name` is inserted as it stands.
    pub name: String,
    /// One line; empty when the command has none.
    pub description: String,
    pub kind: AgentCommandKind,
    /// What the command takes, e.g. `[instructions]`.
    pub argument_hint: Option<String>,
}

/// Where a dropdown entry comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AgentCommandKind {
    /// The agent's own.
    Builtin,
    /// A user or project skill.
    Skill,
    /// A user or project custom command.
    Command,
    /// A plugin's skill or command.
    Plugin,
}

impl From<wire::ChatEvent> for ChatEvent {
    fn from(event: wire::ChatEvent) -> Self {
        Self {
            id: event.id,
            cursor: event.cursor,
            parent: event.parent,
            body: event.body.into(),
        }
    }
}

impl From<EventBody> for ChatBody {
    fn from(body: EventBody) -> Self {
        match body {
            EventBody::Conversation {
                agent,
                conversation_id,
                cwd,
                model,
            } => Self::Conversation {
                agent: agent.name().to_owned(),
                conversation_id,
                cwd,
                model,
            },
            EventBody::User { text, images } => Self::User {
                text,
                images: images.unwrap_or(0),
            },
            EventBody::Assistant { text } => Self::Assistant { text },
            EventBody::Thinking { text } => Self::Thinking { text },
            EventBody::Tool(tool) => Self::Tool { tool: tool.into() },
            EventBody::Question(question) => Self::Question {
                question: question.into(),
            },
            EventBody::Approval(approval) => Self::Approval {
                approval: approval.into(),
            },
            EventBody::ApprovalDone(approval) => Self::ApprovalDone {
                approval: approval.into(),
            },
            EventBody::Notice { kind, text, detail } => Self::Notice {
                kind: kind.into(),
                text,
                detail,
            },
            EventBody::Unknown { entry } => Self::Unknown { entry },
            EventBody::Reset => Self::Reset,
            EventBody::Live => Self::Live,
        }
    }
}

impl From<Tool> for ChatTool {
    fn from(tool: Tool) -> Self {
        Self {
            name: tool.name,
            kind: tool.kind.into(),
            title: tool.title,
            status: tool.status.into(),
            output: tool.output,
            diff: tool.diff,
            truncated: tool.truncated.unwrap_or(false),
        }
    }
}

impl From<ToolKind> for ChatToolKind {
    fn from(kind: ToolKind) -> Self {
        match kind {
            ToolKind::Shell => Self::Shell,
            ToolKind::Edit => Self::Edit,
            ToolKind::Read => Self::Read,
            ToolKind::Search => Self::Search,
            ToolKind::Web => Self::Web,
            ToolKind::Task => Self::Task,
            ToolKind::Other => Self::Other,
        }
    }
}

impl From<ToolStatus> for ChatToolStatus {
    fn from(status: ToolStatus) -> Self {
        match status {
            ToolStatus::Running => Self::Running,
            ToolStatus::Ok => Self::Ok,
            ToolStatus::Error => Self::Error,
        }
    }
}

impl From<Question> for ChatQuestion {
    fn from(question: Question) -> Self {
        Self {
            questions: question.questions.into_iter().map(Into::into).collect(),
            status: question.status.into(),
            output: question.output,
        }
    }
}

impl From<QuestionItem> for ChatQuestionItem {
    fn from(item: QuestionItem) -> Self {
        Self {
            question: item.question,
            header: item.header,
            multi_select: item.multi_select,
            options: item.options.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<QuestionOption> for ChatQuestionOption {
    fn from(option: QuestionOption) -> Self {
        Self {
            label: option.label,
            description: option.description,
        }
    }
}

impl From<Approval> for ChatApproval {
    fn from(approval: Approval) -> Self {
        Self {
            tool: approval.tool,
            title: approval.title,
            detail: approval.detail,
            tool_use_id: approval.tool_use_id,
            cwd: approval.cwd,
        }
    }
}

impl From<NoticeKind> for ChatNoticeKind {
    fn from(kind: NoticeKind) -> Self {
        match kind {
            NoticeKind::Compaction => Self::Compaction,
            NoticeKind::Interrupt => Self::Interrupt,
            NoticeKind::ApiError => Self::ApiError,
            NoticeKind::Summary => Self::Summary,
        }
    }
}

impl From<SlashCommand> for AgentCommand {
    fn from(command: SlashCommand) -> Self {
        Self {
            name: command.name,
            description: command.description,
            kind: match command.kind {
                CommandKind::Builtin => AgentCommandKind::Builtin,
                CommandKind::Skill => AgentCommandKind::Skill,
                CommandKind::Command => AgentCommandKind::Command,
                CommandKind::Plugin => AgentCommandKind::Plugin,
            },
            argument_hint: command.argument_hint,
        }
    }
}
