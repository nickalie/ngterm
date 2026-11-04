//! Claude Code's transcript (`<config>/projects/<project>/<session-id>.jsonl`, read as Claude Code 2.1.317 writes it):
//! one JSON object per line, `type` naming it.
//!
//! - `user` lines carry the prompt (a string, or text and image blocks) or `tool_result` blocks. `isMeta` lines,
//!   local command output and command caveats are Claude's own context and make no event; a `<command-name>` line
//!   shows as the `/command` it ran unless the prompt just before already said so (`/compact` writes both, with only
//!   its compaction between); a `<task-notification>` completes its background task's tool (an `Agent`, or a `Bash`
//!   command run in the background) with its `<result>`, or its `<summary>` when it has none, and a later one for the
//!   same task updates it; `[Request interrupted by user…]` is an interrupt notice; `isCompactSummary` holds a
//!   compaction's summary.
//! - `assistant` lines carry one content block each: `text`, `thinking` (written empty) or `tool_use`. A
//!   `tool_use` is a running tool until the `tool_result` with its id, except `AskUserQuestion`, which is a question.
//!   An `isApiErrorMessage` line is an API error notice.
//! - A background task's launch receipt (`toolUseResult.isAsync` with its `agentId` for an `Agent`,
//!   `toolUseResult.backgroundTaskId` for a `Bash` command) keeps its tool running. Stopping the task (`TaskStop`,
//!   `KillShell` or `KillBash`) usually writes no notification, so a successful stop's result finishes the stopped
//!   task's tool, if still running, as an error with the stop's message; the result names the task in
//!   `toolUseResult.task_id` (or `shell_id`, `bash_id`), else the stop call's input does.
//! - `system` lines: `compact_boundary` is a compaction notice, `api_error` an API error notice; the others are
//!   bookkeeping, as are the line types in [`IGNORED`].
//!
//! A subagent writes its own file, `<session-id>/subagents/agent-<id>.jsonl`, whose `.meta.json` names the parent's
//! `Agent` tool use ([`subagent_parent`]); [`ClaudeParser::subagent`] reads it with every event under that tool.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::event::{DIFF_CAP, OUTPUT_CAP};
use super::{
    AgentKind, ChatEvent, EventBody, Line, NoticeKind, Out, Question, QuestionItem, QuestionOption, Tool, ToolKind,
    ToolStatus, TranscriptParser, cap, compaction, decode_line, finish, non_empty, notice, str_at, tag, title_line,
    unified, untitled_tool,
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

/// The tools that stop a background task.
const STOP_TOOLS: &[&str] = &["TaskStop", "KillShell", "KillBash"];

/// Where a stop call's input, or its result's `toolUseResult`, names the task it stops.
const STOP_TARGETS: &[&str] = &["/task_id", "/shell_id", "/bash_id"];

/// Reads a Claude Code transcript, one line at a time.
#[derive(Debug, Default)]
pub struct ClaudeParser {
    /// The task tool's id when this reads a subagent's file.
    parent: Option<String>,
    /// What names the file, prefixing the ids of events whose line names none ([`ClaudeParser::source`]).
    source: Option<String>,
    /// The conversation announced so far: its id and model.
    conversation: Option<(String, Option<String>)>,
    /// Calls with no result yet, and background tasks (whose tool a later notification updates), by tool use id.
    pending: HashMap<String, EventBody>,
    /// Every call shown so far, by tool use id, so a notification for one no longer pending does not blank it.
    called: HashSet<String>,
    /// Background tasks launched, by task id (a `Bash` command's `backgroundTaskId`, an `Agent`'s `agentId`): the
    /// tool use id of the call that launched each.
    tasks: HashMap<String, String>,
    /// Stop calls with no result yet, by tool use id: the task id each stops.
    stops: HashMap<String, String>,
    /// The prompt just shown, trimmed, so a `<command-name>` line repeating it makes no second bubble. Anything else
    /// shown after it but its compaction clears it.
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

    /// Names the file read `source`: an event whose line names no id of its own is `<source>:line-<cursor>`, not
    /// `line-<cursor>`, so a subagent's offsets do not collide with another file's in one stream.
    #[must_use]
    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
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
        let mut out = Out::new(cursor, self.parent.clone(), self.source.clone());
        match decode_line(line, cursor) {
            Line::Blank => {}
            Line::Unknown(event) => out.push(out.line_id(), event.body),
            Line::Entry(entry) => self.entry(&entry, &mut out),
        }
        self.forget_prompt(&out.events);
        out.events
    }
}

impl ClaudeParser {
    /// The events a decoded line makes, by its `type`.
    fn entry(&mut self, entry: &Value, out: &mut Out) {
        match str_at(entry, "/type").unwrap_or_default() {
            "user" => self.user(entry, out),
            "assistant" => self.assistant(entry, out),
            "system" => system(entry, out),
            "summary" => out.push(
                out.line_id(),
                notice(NoticeKind::Summary, str_at(entry, "/summary").unwrap_or_default(), None),
            ),
            kind if IGNORED.contains(&kind) => {}
            kind => out.push(out.line_id(), EventBody::Unknown { entry: kind.to_owned() }),
        }
    }

    /// Forgets the last prompt once a line shows something after it: only the prompt just before a `<command-name>`
    /// line (with its compaction between, for `/compact`) can repeat it.
    fn forget_prompt(&mut self, events: &[ChatEvent]) {
        let shown = events.iter().rev().find(|event| {
            !matches!(
                event.body,
                EventBody::Conversation { .. }
                    | EventBody::Notice {
                        kind: NoticeKind::Compaction,
                        ..
                    }
            )
        });
        if shown.is_some_and(|event| !matches!(event.body, EventBody::User { .. })) {
            self.last_prompt = None;
        }
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
        let id = str_at(entry, "/uuid").map_or_else(|| out.line_id(), str::to_owned);
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
        let shown = if let Some(command) = command_line(trimmed) {
            if self.last_prompt.take().as_deref() == Some(command.as_str()) {
                return;
            }
            command
        } else {
            self.last_prompt = Some(trimmed.to_owned());
            text.to_owned()
        };
        out.push(
            id,
            EventBody::User {
                text: shown,
                images: (images > 0).then_some(images),
            },
        );
    }

    /// A background task's `<task-notification>`: completes its tool with the task's result, or its summary when it
    /// has none (a background `Bash` command's). The tool stays known, as a resumed task notifies again. A task whose
    /// call is not in the window shows alone; one whose call was shown but is no longer pending makes no event, which
    /// would blank that call's item.
    fn task_notification(&mut self, text: &str, out: &mut Out) {
        let Some(id) = tag(text, "tool-use-id") else { return };
        let status = if tag(text, "status") == Some("completed") {
            ToolStatus::Ok
        } else {
            ToolStatus::Error
        };
        let mut tool = match self.pending.get(id) {
            Some(EventBody::Tool(tool)) => tool.clone(),
            Some(_) => return,
            None if self.called.contains(id) => return,
            None => untitled_tool(ToolKind::Other, status),
        };
        let output = tag(text, "result").or_else(|| tag(text, "summary"));
        finish(&mut tool, status, output.unwrap_or_default());
        self.pending.insert(id.to_owned(), EventBody::Tool(tool.clone()));
        out.push(id, EventBody::Tool(tool));
    }

    /// A `tool_result` block: completes its call, or shows alone when the call is not in the window. A successful
    /// stop's result also finishes the task it stopped.
    fn tool_result(&mut self, entry: &Value, block: &Value, out: &mut Out) {
        let Some(id) = str_at(block, "/tool_use_id") else {
            return;
        };
        let is_error = block.get("is_error").and_then(Value::as_bool) == Some(true);
        let output = content_text(block.get("content").unwrap_or(&Value::Null));
        let stopped = self.stop_target(id, entry);
        self.complete(entry, id, is_error, &output, out);
        if let Some(task) = stopped.filter(|_| !is_error) {
            let message = str_at(entry, "/toolUseResult/message").unwrap_or(&output);
            self.stop_task(&task, message, out);
        }
    }

    /// Completes call `id` with its result.
    fn complete(&mut self, entry: &Value, id: &str, is_error: bool, output: &str, out: &mut Out) {
        let status = if is_error { ToolStatus::Error } else { ToolStatus::Ok };
        let launched = entry.pointer("/toolUseResult/isAsync").and_then(Value::as_bool) == Some(true)
            || non_empty(entry, "/toolUseResult/backgroundTaskId").is_some();
        match self.pending.remove(id) {
            // A background task's launch receipt (an `Agent`'s `isAsync`, a `Bash` command's `backgroundTaskId`): the
            // task runs on until its notification, or until a stop call stops it.
            Some(body) if launched && !is_error => {
                let task = non_empty(entry, "/toolUseResult/backgroundTaskId")
                    .or_else(|| non_empty(entry, "/toolUseResult/agentId"));
                if let Some(task) = task {
                    self.tasks.insert(task.to_owned(), id.to_owned());
                }
                self.pending.insert(id.to_owned(), body);
            }
            Some(EventBody::Question(mut question)) => {
                question.status = status;
                question.output = Some(cap(output, OUTPUT_CAP).0);
                out.push(id, EventBody::Question(question));
            }
            Some(EventBody::Tool(mut tool)) => {
                finish(&mut tool, status, output);
                out.push(id, EventBody::Tool(tool));
            }
            _ => {
                let mut tool = untitled_tool(ToolKind::Other, status);
                finish(&mut tool, status, output);
                out.push(id, EventBody::Tool(tool));
            }
        }
    }

    /// The task a stop call's result stops: the one its `toolUseResult` names, else the one its input named. A
    /// result whose call is not in the window counts when its `toolUseResult` names a task; one of a shown call that
    /// is no stop does not.
    fn stop_target(&mut self, id: &str, entry: &Value) -> Option<String> {
        let remembered = self.stops.remove(id);
        if remembered.is_none() && self.called.contains(id) {
            return None;
        }
        let receipt = entry.get("toolUseResult").unwrap_or(&Value::Null);
        STOP_TARGETS
            .iter()
            .find_map(|pointer| non_empty(receipt, pointer))
            .map(str::to_owned)
            .or(remembered)
    }

    /// Finishes the call that launched background task `task`, if it is still running, as stopped: an error whose
    /// output is the stop's `message`. The call stays known, so a notification that still comes updates it.
    fn stop_task(&mut self, task: &str, message: &str, out: &mut Out) {
        let Some(id) = self.tasks.get(task) else { return };
        let Some(EventBody::Tool(tool)) = self.pending.get(id) else {
            return;
        };
        if tool.status != ToolStatus::Running {
            return;
        }
        let mut tool = tool.clone();
        let id = id.clone();
        finish(&mut tool, ToolStatus::Error, message);
        self.tasks.remove(task);
        self.pending.insert(id.clone(), EventBody::Tool(tool.clone()));
        out.push(id, EventBody::Tool(tool));
    }

    fn assistant(&mut self, entry: &Value, out: &mut Out) {
        self.conversation(entry, str_at(entry, "/message/model"), out);
        let uuid = str_at(entry, "/uuid").map_or_else(|| out.line_id(), str::to_owned);
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
        if STOP_TOOLS.contains(&name)
            && let Some(task) = STOP_TARGETS.iter().find_map(|pointer| non_empty(input, pointer))
        {
            self.stops.insert(id.to_owned(), task.to_owned());
        }
        self.pending.insert(id.to_owned(), body.clone());
        self.called.insert(id.to_owned());
        out.push(id, body);
    }
}

fn system(entry: &Value, out: &mut Out) {
    let id = str_at(entry, "/uuid").map_or_else(|| out.line_id(), str::to_owned);
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

pub(crate) fn tool_kind(name: &str) -> ToolKind {
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
pub(crate) fn tool_title(kind: ToolKind, name: &str, input: &Value) -> String {
    title_line(tool_subject(kind, input).unwrap_or(name))
}

/// The input field that says what a call does: its command, path, pattern, URL or query, in full.
pub(crate) fn tool_subject(kind: ToolKind, input: &Value) -> Option<&str> {
    let fields: &[&str] = match kind {
        ToolKind::Shell => &["command", "bash_id", "shell_id"],
        ToolKind::Edit | ToolKind::Read => &["file_path", "notebook_path"],
        ToolKind::Search => &["pattern", "path"],
        ToolKind::Web => &["url", "query"],
        ToolKind::Task => &["description", "prompt"],
        ToolKind::Other => &[],
    };
    fields
        .iter()
        .find_map(|field| input.get(*field).and_then(Value::as_str))
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

/// `/name args` from a `<command-name>` line, or `None` for any other text (a prompt quoting the markup among its
/// words is a prompt). Some Claude Code versions write `<command-message>` first.
fn command_line(text: &str) -> Option<String> {
    if !text.starts_with("<command-name>") && !text.starts_with("<command-message>") {
        return None;
    }
    let name = tag(text, "command-name")?;
    let args = tag(text, "command-args").unwrap_or_default().trim();
    Some(if args.is_empty() {
        name.to_owned()
    } else {
        format!("{name} {args}")
    })
}
