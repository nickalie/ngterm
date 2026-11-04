//! Claude Code's transcript (`<config>/projects/<project>/<session-id>.jsonl`, read as Claude Code 2.1.317 writes it):
//! one JSON object per line, `type` naming it.
//!
//! - `user` lines carry the prompt (a string, or text and image blocks) or `tool_result` blocks. `isMeta` lines,
//!   local command output and command caveats are Claude's own context and make no event; a `<command-name>` line
//!   shows as the `/command` it ran unless the prompt just before already said so; a `<task-notification>` completes
//!   its background task's tool; `[Request interrupted by user…]` is an interrupt notice; `isCompactSummary` holds a
//!   compaction's summary.
//! - `assistant` lines carry one content block each: `text`, `thinking` (written empty) or `tool_use`. A
//!   `tool_use` is a running tool until the `tool_result` with its id, except `AskUserQuestion`, which is a question.
//!   An `isApiErrorMessage` line is an API error notice.
//! - `system` lines: `compact_boundary` is a compaction notice, `api_error` an API error notice; the others are
//!   bookkeeping, as are the line types in [`IGNORED`].
//!
//! A subagent writes its own file, `<session-id>/subagents/agent-<id>.jsonl`, whose `.meta.json` names the parent's
//! `Agent` tool use ([`subagent_parent`]); [`ClaudeParser::subagent`] reads it with every event under that tool.

use std::collections::HashMap;

use serde_json::Value;

use super::event::{DIFF_CAP, OUTPUT_CAP};
use super::{
    AgentKind, ChatEvent, EventBody, Line, NoticeKind, Question, QuestionItem, QuestionOption, Tool, ToolKind,
    ToolStatus, TranscriptParser, cap, decode_line, line_id, title_line,
};

/// Line types the chat has no use for (Task 1's fixture README lists them).
pub const IGNORED: &[&str] = &[
    "ai-title",
    "api-request",
    "api-request-blob",
    "api-request-shape",
    "atis-latch",
    "attachment",
    "cost-state",
    "custom-title",
    "file-history-delta",
    "file-history-snapshot",
    "last-prompt",
    "mode",
    "permission-mode",
    "progress",
    "queue-operation",
    "session-owner",
    "tools-used",
];

/// `system` subtypes the chat has no use for.
const IGNORED_SYSTEM: &[&str] = &["stop_hook_summary", "turn_duration", "local_command", "informational"];

/// The id of the one `conversation` item a stream has.
const CONVERSATION_ID: &str = "conversation";

/// The tool Claude asks the user questions with.
const ASK_USER_QUESTION: &str = "AskUserQuestion";

/// Reads a Claude Code transcript, one line at a time.
#[derive(Debug, Default)]
pub struct ClaudeParser {
    /// The task tool's id when this reads a subagent's file.
    parent: Option<String>,
    /// The conversation announced so far: its id and model.
    conversation: Option<(String, Option<String>)>,
    /// Calls with no result yet (and background tasks with no notification yet), by tool use id.
    pending: HashMap<String, EventBody>,
    /// The last prompt shown, so a `<command-name>` line repeating it makes no second bubble.
    last_prompt: Option<String>,
}

impl ClaudeParser {
    /// A parser for a conversation's main transcript.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A parser for a subagent's transcript: every event is parented to `parent`, the task tool that ran it, and no
    /// `conversation` is announced.
    #[must_use]
    pub fn subagent(parent: impl Into<String>) -> Self {
        Self {
            parent: Some(parent.into()),
            ..Self::default()
        }
    }
}

/// The parent task's tool use id named by a subagent's `agent-<id>.meta.json` (`toolUseId`), if it names one.
#[must_use]
pub fn subagent_parent(meta_json: &[u8]) -> Option<String> {
    let meta: Value = serde_json::from_slice(meta_json).ok()?;
    meta.get("toolUseId")?.as_str().map(str::to_owned)
}

impl TranscriptParser for ClaudeParser {
    fn parse_line(&mut self, line: &[u8], cursor: u64) -> Vec<ChatEvent> {
        let entry = match decode_line(line, cursor) {
            Line::Blank => return Vec::new(),
            Line::Unknown(event) => return vec![event],
            Line::Entry(entry) => entry,
        };
        let mut out = Out {
            cursor,
            parent: self.parent.clone(),
            events: Vec::new(),
        };
        match str_at(&entry, "/type").unwrap_or_default() {
            "user" => self.user(&entry, &mut out),
            "assistant" => self.assistant(&entry, &mut out),
            "system" => system(&entry, &mut out),
            "summary" => out.push(
                line_id(cursor),
                notice(
                    NoticeKind::Summary,
                    str_at(&entry, "/summary").unwrap_or_default(),
                    None,
                ),
            ),
            kind if IGNORED.contains(&kind) => {}
            kind => out.push(line_id(cursor), EventBody::Unknown { entry: kind.to_owned() }),
        }
        out.events
    }
}

/// The events one line makes.
struct Out {
    cursor: u64,
    parent: Option<String>,
    events: Vec<ChatEvent>,
}

impl Out {
    fn push(&mut self, id: impl Into<String>, body: EventBody) {
        self.events.push(ChatEvent {
            id: id.into(),
            cursor: self.cursor,
            parent: self.parent.clone(),
            body,
        });
    }
}

impl ClaudeParser {
    /// Announces the conversation when this line names a new one, or a model it did not have.
    fn conversation(&mut self, entry: &Value, model: Option<&str>, out: &mut Out) {
        let (None, Some(id)) = (&self.parent, str_at(entry, "/sessionId")) else {
            return;
        };
        let model = model.filter(|model| !model.starts_with('<')).map(str::to_owned);
        let changed = match &mut self.conversation {
            Some((known, known_model)) if known == id => {
                let newer = model.is_some() && *known_model != model;
                if newer {
                    known_model.clone_from(&model);
                }
                newer
            }
            _ => {
                self.conversation = Some((id.to_owned(), model.clone()));
                true
            }
        };
        if changed {
            let cwd = str_at(entry, "/cwd").map(str::to_owned);
            let body = EventBody::Conversation {
                agent: AgentKind::Claude,
                conversation_id: id.to_owned(),
                cwd,
                model,
            };
            out.push(CONVERSATION_ID, body);
        }
    }

    fn user(&mut self, entry: &Value, out: &mut Out) {
        self.conversation(entry, None, out);
        if entry.get("isMeta").and_then(Value::as_bool) == Some(true) {
            return;
        }
        let id = str_at(entry, "/uuid").map_or_else(|| line_id(out.cursor), str::to_owned);
        let content = entry.pointer("/message/content").unwrap_or(&Value::Null);
        if entry.get("isCompactSummary").and_then(Value::as_bool) == Some(true) {
            // The summary follows its boundary: update the boundary's notice.
            let boundary = str_at(entry, "/parentUuid").map_or(id, str::to_owned);
            out.push(boundary, compaction(Some(content_text(content))));
            return;
        }
        match content {
            Value::String(text) => self.prompt(text, 0, id, out),
            Value::Array(blocks) => self.user_blocks(entry, blocks, id, out),
            _ => {}
        }
    }

    /// A `user` line's blocks: tool results, then the prompt its text and images make.
    fn user_blocks(&mut self, entry: &Value, blocks: &[Value], id: String, out: &mut Out) {
        let mut texts = Vec::new();
        let mut images = 0;
        for block in blocks {
            match str_at(block, "/type").unwrap_or_default() {
                "tool_result" => self.tool_result(entry, block, out),
                "text" => texts.push(str_at(block, "/text").unwrap_or_default()),
                "image" => images += 1,
                _ => {}
            }
        }
        if !texts.is_empty() || images > 0 {
            self.prompt(&texts.join("\n"), images, id, out);
        }
    }

    /// The prompt a `user` line's text makes, if it is one: Claude's own markup makes a notice, a task's result or
    /// nothing.
    fn prompt(&mut self, text: &str, images: u32, id: String, out: &mut Out) {
        let trimmed = text.trim();
        if let Some(reason) = trimmed
            .strip_prefix("[Request interrupted by user")
            .and_then(|r| r.strip_suffix(']'))
        {
            out.push(
                id,
                notice(
                    NoticeKind::Interrupt,
                    &format!("Request interrupted by user{reason}"),
                    None,
                ),
            );
            return;
        }
        if trimmed.starts_with("<task-notification>") {
            self.task_notification(trimmed, out);
            return;
        }
        if trimmed.starts_with("<local-command-") {
            return;
        }
        let shown = match command_line(trimmed) {
            Some(command) if self.last_prompt.as_deref() == Some(command.as_str()) => return,
            Some(command) => command,
            None => text.to_owned(),
        };
        self.last_prompt = Some(shown.clone());
        out.push(
            id,
            EventBody::User {
                text: shown,
                images: (images > 0).then_some(images),
            },
        );
    }

    /// A background task's `<task-notification>`: completes its tool with the task's result.
    fn task_notification(&mut self, text: &str, out: &mut Out) {
        let Some(id) = tag(text, "tool-use-id") else { return };
        let status = if tag(text, "status") == Some("completed") {
            ToolStatus::Ok
        } else {
            ToolStatus::Error
        };
        let mut tool = match self.pending.remove(id) {
            Some(EventBody::Tool(tool)) => tool,
            _ => Tool {
                name: String::new(),
                kind: ToolKind::Task,
                title: String::new(),
                status,
                output: None,
                diff: None,
                truncated: None,
            },
        };
        finish(&mut tool, status, tag(text, "result").unwrap_or_default());
        out.push(id, EventBody::Tool(tool));
    }

    /// A `tool_result` block: completes its call, or shows alone when the call is not in the window.
    fn tool_result(&mut self, entry: &Value, block: &Value, out: &mut Out) {
        let Some(id) = str_at(block, "/tool_use_id") else {
            return;
        };
        let is_error = block.get("is_error").and_then(Value::as_bool) == Some(true);
        let status = if is_error { ToolStatus::Error } else { ToolStatus::Ok };
        let output = content_text(block.get("content").unwrap_or(&Value::Null));
        let launched = entry.pointer("/toolUseResult/isAsync").and_then(Value::as_bool) == Some(true);
        match self.pending.remove(id) {
            // A background task's launch receipt: the task runs on until its notification.
            Some(body) if launched && !is_error => {
                self.pending.insert(id.to_owned(), body);
            }
            Some(EventBody::Question(mut question)) => {
                question.status = status;
                question.output = Some(cap(&output, OUTPUT_CAP).0);
                out.push(id, EventBody::Question(question));
            }
            Some(EventBody::Tool(mut tool)) => {
                finish(&mut tool, status, &output);
                out.push(id, EventBody::Tool(tool));
            }
            _ => {
                let mut tool = Tool {
                    name: String::new(),
                    kind: ToolKind::Other,
                    title: String::new(),
                    status,
                    output: None,
                    diff: None,
                    truncated: None,
                };
                finish(&mut tool, status, &output);
                out.push(id, EventBody::Tool(tool));
            }
        }
    }

    fn assistant(&mut self, entry: &Value, out: &mut Out) {
        self.conversation(entry, str_at(entry, "/message/model"), out);
        let uuid = str_at(entry, "/uuid").map_or_else(|| line_id(out.cursor), str::to_owned);
        let blocks = entry
            .pointer("/message/content")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        if entry.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) {
            let text = blocks
                .iter()
                .filter_map(|block| str_at(block, "/text"))
                .collect::<Vec<_>>()
                .join("\n");
            out.push(uuid, notice(NoticeKind::ApiError, &text, None));
            return;
        }
        for (index, block) in blocks.iter().enumerate() {
            let id = if index == 0 {
                uuid.clone()
            } else {
                format!("{uuid}#{index}")
            };
            match str_at(block, "/type").unwrap_or_default() {
                "text" => out.push(
                    id,
                    EventBody::Assistant {
                        text: str_at(block, "/text").unwrap_or_default().into(),
                    },
                ),
                "thinking" => {
                    out.push(
                        id,
                        EventBody::Thinking {
                            text: str_at(block, "/thinking").unwrap_or_default().into(),
                        },
                    );
                }
                "redacted_thinking" => out.push(id, EventBody::Thinking { text: String::new() }),
                "tool_use" | "server_tool_use" => self.tool_use(block, out),
                kind => out.push(
                    id,
                    EventBody::Unknown {
                        entry: format!("assistant:{kind}"),
                    },
                ),
            }
        }
    }

    /// A `tool_use` block: a running tool, or an open question.
    fn tool_use(&mut self, block: &Value, out: &mut Out) {
        let Some(id) = str_at(block, "/id") else { return };
        let name = str_at(block, "/name").unwrap_or_default();
        let input = block.get("input").unwrap_or(&Value::Null);
        let body = if name == ASK_USER_QUESTION {
            EventBody::Question(question(input))
        } else {
            EventBody::Tool(running_tool(name, input))
        };
        self.pending.insert(id.to_owned(), body.clone());
        out.push(id, body);
    }
}

fn system(entry: &Value, out: &mut Out) {
    let id = str_at(entry, "/uuid").map_or_else(|| line_id(out.cursor), str::to_owned);
    match str_at(entry, "/subtype").unwrap_or_default() {
        "compact_boundary" => out.push(id, compaction(None)),
        "api_error" => {
            let text = ["/content", "/error/message", "/error/error/message"]
                .iter()
                .find_map(|pointer| str_at(entry, pointer))
                .unwrap_or("API error");
            out.push(id, notice(NoticeKind::ApiError, text, None));
        }
        subtype if IGNORED_SYSTEM.contains(&subtype) => {}
        subtype => out.push(
            id,
            EventBody::Unknown {
                entry: format!("system:{subtype}"),
            },
        ),
    }
}

fn notice(kind: NoticeKind, text: &str, detail: Option<String>) -> EventBody {
    EventBody::Notice {
        kind,
        text: text.to_owned(),
        detail,
    }
}

/// A compaction notice; the boundary line carries no summary, the summary line after it does.
fn compaction(summary: Option<String>) -> EventBody {
    notice(NoticeKind::Compaction, "Conversation compacted", summary)
}

/// A tool call as it shows before its result.
fn running_tool(name: &str, input: &Value) -> Tool {
    let kind = tool_kind(name);
    let (diff, truncated) = match edit_diff(name, input) {
        Some(diff) => {
            let (diff, cut) = cap(&diff, DIFF_CAP);
            (Some(diff), cut.then_some(true))
        }
        None => (None, None),
    };
    Tool {
        name: name.to_owned(),
        kind,
        title: tool_title(kind, name, input),
        status: ToolStatus::Running,
        output: None,
        diff,
        truncated,
    }
}

/// A call's result: its status and its output's first [`OUTPUT_CAP`] bytes.
fn finish(tool: &mut Tool, status: ToolStatus, output: &str) {
    tool.status = status;
    let (output, cut) = cap(output, OUTPUT_CAP);
    tool.output = (!output.is_empty()).then_some(output);
    if cut {
        tool.truncated = Some(true);
    }
}

fn tool_kind(name: &str) -> ToolKind {
    match name {
        "Bash" | "BashOutput" | "KillShell" | "KillBash" | "PowerShell" => ToolKind::Shell,
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => ToolKind::Edit,
        "Read" | "NotebookRead" => ToolKind::Read,
        "Grep" | "Glob" | "LS" => ToolKind::Search,
        "WebFetch" | "WebSearch" => ToolKind::Web,
        "Agent" | "Task" => ToolKind::Task,
        _ => ToolKind::Other,
    }
}

/// A call's one-line title: the input field that says what it does, or the tool's name.
fn tool_title(kind: ToolKind, name: &str, input: &Value) -> String {
    let fields: &[&str] = match kind {
        ToolKind::Shell => &["command", "bash_id", "shell_id"],
        ToolKind::Edit | ToolKind::Read => &["file_path", "notebook_path"],
        ToolKind::Search => &["pattern", "path"],
        ToolKind::Web => &["url", "query"],
        ToolKind::Task => &["description", "prompt"],
        ToolKind::Other => &[],
    };
    let field = fields
        .iter()
        .find_map(|field| input.get(*field).and_then(Value::as_str));
    title_line(field.unwrap_or(name))
}

/// The unified diff an edit tool's input describes. Hunk line numbers count from the edited text, not the file: the
/// input does not say where in the file it is.
fn edit_diff(name: &str, input: &Value) -> Option<String> {
    let path = str_at(input, "/file_path")
        .or_else(|| str_at(input, "/notebook_path"))
        .unwrap_or_default();
    match name {
        "Edit" => Some(unified(
            path,
            str_at(input, "/old_string")?,
            str_at(input, "/new_string")?,
        )),
        "MultiEdit" => {
            let edits = input.get("edits")?.as_array()?;
            let hunks = edits.iter().filter_map(|edit| {
                Some(unified(
                    path,
                    str_at(edit, "/old_string")?,
                    str_at(edit, "/new_string")?,
                ))
            });
            Some(hunks.collect())
        }
        "Write" => Some(unified(path, "", str_at(input, "/content")?)),
        "NotebookEdit" => Some(unified(path, "", str_at(input, "/new_source")?)),
        _ => None,
    }
}

fn unified(path: &str, old: &str, new: &str) -> String {
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .missing_newline_hint(false)
        .header(path, path)
        .to_string()
}

/// An `AskUserQuestion` input's questions.
fn question(input: &Value) -> Question {
    let questions = input
        .get("questions")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let questions = questions
        .iter()
        .map(|item| QuestionItem {
            question: str_at(item, "/question").unwrap_or_default().to_owned(),
            header: str_at(item, "/header").map(str::to_owned),
            multi_select: item.get("multiSelect").and_then(Value::as_bool).unwrap_or(false),
            options: item
                .get("options")
                .and_then(Value::as_array)
                .map_or_else(Vec::new, |options| {
                    options
                        .iter()
                        .map(|option| QuestionOption {
                            label: str_at(option, "/label").unwrap_or_default().to_owned(),
                            description: str_at(option, "/description").map(str::to_owned),
                        })
                        .collect()
                }),
        })
        .collect();
    Question {
        questions,
        status: ToolStatus::Running,
        output: None,
    }
}

/// A message or tool result's text: the string, or its text blocks joined.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| str_at(block, "/text"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// `/name args` from a `<command-name>` line, or `None` for any other text.
fn command_line(text: &str) -> Option<String> {
    let name = tag(text, "command-name")?;
    let args = tag(text, "command-args").unwrap_or_default().trim();
    Some(if args.is_empty() {
        name.to_owned()
    } else {
        format!("{name} {args}")
    })
}

/// The text between `<name>` and `</name>`.
fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let start = text.find(&format!("<{name}>"))? + name.len() + 2;
    let end = text[start..].find(&format!("</{name}>"))? + start;
    Some(text[start..end].trim())
}

fn str_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}
