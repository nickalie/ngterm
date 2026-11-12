//! Codex's rollout (`<codex home>/sessions/YYYY/MM/DD/rollout-<time>-<thread id>.jsonl`, read as Codex CLI 0.166 and
//! 0.167 write it): one JSON object per line, `type` naming it and `payload` holding it.
//!
//! - `session_meta` names the thread (the conversation) and its cwd; `turn_context` carries the model.
//! - `event_msg` `item_completed` items are the typed record: `UserMessage`, `AgentMessage`, `Reasoning` (its summary
//!   titles), the tool items (`CommandExecution`, `FileChange`, `Extension`, `CollabAgentToolCall`) and
//!   `ContextCompaction`. Turns are bracketed by `task_started` and `task_complete` or `turn_aborted`; `error` is an API
//!   error notice.
//! - `response_item` lines are what the model saw and said. An assistant message or a reasoning item repeats an
//!   `item_completed` one with the same id, so each id shows once, from whichever line comes first. User messages are
//!   not shown from here: they mix the prompt with Codex's own context, and the `UserMessage` item has a different id.
//!   A `<subagent_notification>` user message finishes the subagent's task tool, the way Claude's
//!   `<task-notification>` does.
//! - Every tool runs through code mode: a `custom_tool_call` named `exec` holds JavaScript (`tools.exec_command(…)`,
//!   `tools.apply_patch(…)`, …) and is a running tool until its `custom_tool_call_output`. A tool item completed while
//!   the call runs takes the call's place (same id) with what it did; a call no item claims finishes with the script's
//!   output (`Script completed…`, or `aborted by user…` when a prompt was denied). `function_call` lines (a tool run
//!   outside code mode) read the same way.
//! - A script that outlasts its first wait (a command waiting for its approval, a long one) yields: its output says
//!   `Script running with cell ID N…` and the call runs on. A `wait` function call on that cell is plumbing and shows
//!   no row: each output carries only what the script printed since the last, so once the script ended, all of it
//!   finishes the call unless a tool item took the call's place.
//! - `request_user_input` (Codex's question picker, Plan mode) is a question, not a tool: open until its output, whose
//!   `answers` (keyed by each question's `id`; a note typed on an option as a `user_note: …` answer) complete it. A
//!   turn that only asked ends with an empty assistant message, which shows nothing.
//! - A spawned subagent's task runs on after `spawn_agent` returns, until its `<subagent_notification>` or a later
//!   collab item reporting a final state for it (a `wait` or a `close_agent`).
//! - A call still running when its turn ends (`task_complete`, `turn_aborted`) or when another turn starts (after a
//!   crash and `codex resume`) finishes as an error: it will get no output.
//!
//! A subagent writes its own rollout, which starts with the parent's history up to the spawn
//! (`subagent_history_start_ordinal` on its `session_meta`); [`CodexParser::subagent`] skips that part and reads the
//! rest under the task tool [`CodexParser::task_for_thread`] names. An item a subagent's thread completes inside the
//! parent's rollout folds under the same tool; a tool item of its never takes the place of the parent's own calls.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::event::{DIFF_CAP, OUTPUT_CAP};
use super::{
    AgentKind, ChatEvent, EventBody, Line, NoticeKind, Out, Question, Tool, ToolKind, ToolStatus, TranscriptParser,
    cap, compaction, decode_line, finish, non_empty, notice, question, str_at, tag, title_line, unified, untitled_tool,
};

/// Line types the chat has no use for.
pub const IGNORED: &[&str] = &["token_usage_record", "world_state", "retained_context"];

/// `event_msg` types the chat has no use for.
const IGNORED_EVENTS: &[&str] = &["token_count", "thread_settings_applied"];

/// `response_item` types the chat has no use for.
const IGNORED_RESPONSES: &[&str] = &["additional_tools"];

/// The id of the one `conversation` item a stream has.
const CONVERSATION_ID: &str = "conversation";

/// The output of a call whose turn ended before it wrote one.
const INTERRUPTED: &str = "Interrupted";

/// The output of a call whose turn completed, or was left behind by another, before it wrote one.
const NO_RESULT: &str = "No result was recorded";

/// What `exec`'s output starts with when the script ran to its end.
const SCRIPT_COMPLETED: &str = "Script completed";

/// What `exec`'s (or `wait`'s) output starts with when the script yielded before its end, followed by the cell's id.
const SCRIPT_RUNNING: &str = "Script running with cell ID ";

/// The code-mode tool that waits on a yielded script's cell.
const WAIT: &str = "wait";

/// The tool Codex asks the user questions with.
const REQUEST_USER_INPUT: &str = "request_user_input";

/// What a note typed on a picked option starts with among a question's answers.
const USER_NOTE: &str = "user_note: ";

/// Reads a Codex rollout, one line at a time.
#[derive(Debug, Default)]
pub struct CodexParser {
    /// The task tool's id when this reads a subagent's rollout.
    parent: Option<String>,
    /// What names the file, prefixing the ids of events whose line names none ([`CodexParser::source`]).
    source: Option<String>,
    /// The rollout's own thread, from its first `session_meta`.
    thread: Option<String>,
    /// The first line (by `ordinal`) of a subagent's own history; the lines before it are the parent's.
    history_start: Option<u64>,
    /// The conversation announced so far.
    conversation: Option<Conversation>,
    /// The turn running now.
    turn: Option<String>,
    /// Calls with no output yet, oldest first.
    calls: Vec<Call>,
    /// Every tool shown so far, by event id, as last shown.
    tools: HashMap<String, Tool>,
    /// Spawned subagents, by thread id (and agent path): the event id of the task tool that spawned each.
    agents: HashMap<String, String>,
    /// Messages and reasoning shown so far, by id: `item_completed` and `response_item` both write them.
    shown: HashSet<String>,
    /// Compactions shown so far, by turn.
    compactions: HashSet<String>,
    /// Yielded scripts, by cell id: the call that started each.
    cells: HashMap<String, String>,
    /// What yielded scripts printed so far, by the call that started each.
    printed: HashMap<String, String>,
    /// `wait` calls with no output yet, by call id: the call that started the cell waited on, when it is known.
    waits: HashMap<String, Option<String>>,
    /// Questions shown so far, by call id, with each question's id (what its answers are keyed by).
    questions: HashMap<String, (Question, Vec<String>)>,
}

#[derive(Debug, Clone)]
struct Conversation {
    id: String,
    cwd: Option<String>,
    model: Option<String>,
}

/// A tool call with no output yet.
#[derive(Debug)]
struct Call {
    /// The call id, which is also its event id.
    id: String,
    turn: Option<String>,
    /// What the tools the call runs do, for matching the items they complete.
    kinds: Vec<ToolKind>,
    /// The shell command the call runs, when it runs one.
    command: Option<String>,
    /// A tool item has taken the call's place: its output adds nothing.
    claimed: bool,
}

/// Whose an `item_completed` is.
enum Owner {
    /// This rollout's thread.
    Own,
    /// A subagent's thread: its items fold under this task tool.
    Agent(String),
    /// Another thread's.
    Foreign,
}

impl CodexParser {
    /// A parser for a conversation's rollout.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A parser for a subagent's rollout: every event is parented to `parent`, the task tool that spawned it, no
    /// `conversation` is announced, and the parent's history it starts with is skipped.
    #[must_use]
    pub fn subagent(parent: impl Into<String>) -> Self {
        Self {
            parent: Some(parent.into()),
            ..Self::default()
        }
    }

    /// Names the rollout read `source`: an event whose line names no id of its own is `<source>:line-<cursor>`, not
    /// `line-<cursor>`, so a subagent's offsets do not collide with another file's in one stream.
    #[must_use]
    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// The event id of the task tool that spawned subagent `thread`, once the line spawning it was read: the parent
    /// [`CodexParser::subagent`] reads that thread's rollout under.
    #[must_use]
    pub fn task_for_thread(&self, thread: &str) -> Option<&str> {
        self.agents.get(thread).map(String::as_str)
    }

    /// Every subagent spawned so far: its thread id (or agent path) and the event id of the task tool it runs under,
    /// for finding the subagents' rollouts.
    pub fn spawned_threads(&self) -> impl Iterator<Item = (&str, &str)> {
        self.agents
            .iter()
            .map(|(thread, task)| (thread.as_str(), task.as_str()))
    }
}

impl TranscriptParser for CodexParser {
    fn parse_line(&mut self, line: &[u8], cursor: u64) -> Vec<ChatEvent> {
        let mut out = Out::new(cursor, self.parent.clone(), self.source.clone());
        match decode_line(line, cursor) {
            Line::Blank => {}
            Line::Unknown(event) => out.push(out.line_id(), event.body),
            Line::Entry(entry) => self.entry(&entry, &mut out),
        }
        out.events
    }
}

impl CodexParser {
    /// The events a decoded line makes, by its `type`.
    fn entry(&mut self, entry: &Value, out: &mut Out) {
        if self.inherited(entry) {
            return;
        }
        let payload = entry.get("payload").unwrap_or(&Value::Null);
        match str_at(entry, "/type").unwrap_or_default() {
            "session_meta" => self.session_meta(payload, out),
            "turn_context" => self.model(str_at(payload, "/model"), out),
            "event_msg" => self.event(payload, out),
            "response_item" => self.response(payload, out),
            "compacted" => {
                let key = self.turn.clone().unwrap_or_else(|| out.line_id());
                let summary = non_empty(payload, "/message").map(str::to_owned);
                if self.compactions.insert(key.clone()) || summary.is_some() {
                    out.push(format!("compaction-{key}"), compaction(summary));
                }
            }
            kind if IGNORED.contains(&kind) => {}
            kind => out.push(out.line_id(), EventBody::Unknown { entry: kind.to_owned() }),
        }
    }

    /// A line of the parent's history a subagent's rollout starts with.
    fn inherited(&self, entry: &Value) -> bool {
        let ordinal = entry.get("ordinal").and_then(Value::as_u64);
        matches!((self.history_start, ordinal), (Some(start), Some(ordinal)) if ordinal < start)
    }

    /// The rollout's first `session_meta` names its thread; a later one (a subagent's copy of its parent's) is
    /// history.
    fn session_meta(&mut self, payload: &Value, out: &mut Out) {
        let Some(id) = str_at(payload, "/id") else { return };
        if self.thread.is_some() {
            return;
        }
        self.thread = Some(id.to_owned());
        self.history_start = payload.get("subagent_history_start_ordinal").and_then(Value::as_u64);
        if self.parent.is_none() {
            let conversation = Conversation {
                id: id.to_owned(),
                cwd: str_at(payload, "/cwd").map(str::to_owned),
                model: None,
            };
            announce(&conversation, out);
            self.conversation = Some(conversation);
        }
    }

    /// Announces the conversation again when its model becomes known or changes.
    fn model(&mut self, model: Option<&str>, out: &mut Out) {
        let (Some(conversation), Some(model)) = (&mut self.conversation, model) else {
            return;
        };
        if conversation.model.as_deref() != Some(model) {
            conversation.model = Some(model.to_owned());
            announce(conversation, out);
        }
    }

    fn event(&mut self, payload: &Value, out: &mut Out) {
        let turn = str_at(payload, "/turn_id");
        match str_at(payload, "/type").unwrap_or_default() {
            "item_completed" => self.item(payload, out),
            "task_started" => {
                self.end_calls(|call| call.turn.as_deref() != turn, NO_RESULT, out);
                self.turn = turn.map(str::to_owned);
            }
            "task_complete" => self.end_calls(|call| call.turn.as_deref() == turn, NO_RESULT, out),
            "turn_aborted" => {
                let text = match str_at(payload, "/reason").unwrap_or("interrupted") {
                    "interrupted" => "Turn interrupted by user".to_owned(),
                    reason => format!("Turn aborted ({reason})"),
                };
                out.push(out.line_id(), notice(NoticeKind::Interrupt, &text, None));
                self.end_calls(|call| call.turn.as_deref() == turn, INTERRUPTED, out);
            }
            "error" => {
                let text = str_at(payload, "/message").unwrap_or("API error");
                out.push(out.line_id(), notice(NoticeKind::ApiError, text, None));
            }
            kind if IGNORED_EVENTS.contains(&kind) => {}
            kind => out.push(
                out.line_id(),
                EventBody::Unknown {
                    entry: format!("event_msg:{kind}"),
                },
            ),
        }
    }

    /// Finishes the calls `ended` picks, which will get no output, as errors with `why`. A call a tool item claimed
    /// keeps that item's result, and a subagent's task runs on.
    fn end_calls(&mut self, ended: impl Fn(&Call) -> bool, why: &str, out: &mut Out) {
        let (gone, kept): (Vec<Call>, Vec<Call>) = std::mem::take(&mut self.calls).into_iter().partition(ended);
        self.calls = kept;
        for call in gone {
            self.printed.remove(&call.id);
            if let Some((asked, _)) = self.questions.get_mut(&call.id) {
                if asked.status == ToolStatus::Running {
                    asked.status = ToolStatus::Error;
                    asked.output = Some(why.to_owned());
                    out.push(call.id, EventBody::Question(asked.clone()));
                }
                continue;
            }
            let is_task = self.agents.values().any(|task| *task == call.id);
            let Some(tool) = self.tools.get_mut(&call.id) else {
                continue;
            };
            if tool.status == ToolStatus::Running && !is_task {
                finish(tool, ToolStatus::Error, why);
                out.push(call.id, EventBody::Tool(tool.clone()));
            }
        }
    }

    /// An `item_completed` event's item.
    fn item(&mut self, payload: &Value, out: &mut Out) {
        let item = payload.get("item").unwrap_or(&Value::Null);
        let own = match self.owner(str_at(payload, "/thread_id")) {
            Owner::Own => true,
            Owner::Agent(task) => {
                out.parent = Some(task);
                false
            }
            Owner::Foreign => return,
        };
        let id = str_at(item, "/id").map_or_else(|| out.line_id(), str::to_owned);
        match str_at(item, "/type").unwrap_or_default() {
            "UserMessage" => user_message(item, id, out),
            "AgentMessage" => self.reply(id, texts(item, "/content", "Text"), out),
            "Reasoning" => {
                let text = strings(item.get("summary_text")).join("\n");
                self.once(id, EventBody::Thinking { text }, out);
            }
            "ContextCompaction" => {
                let key = str_at(payload, "/turn_id").map_or(id, str::to_owned);
                if self.compactions.insert(key.clone()) {
                    out.push(format!("compaction-{key}"), compaction(None));
                }
            }
            "CommandExecution" | "FileChange" | "Extension" | "CollabAgentToolCall" => {
                self.tool_item(item, id, own, out);
            }
            kind => out.push(
                id,
                EventBody::Unknown {
                    entry: format!("item:{kind}"),
                },
            ),
        }
    }

    fn owner(&self, thread: Option<&str>) -> Owner {
        match (thread, &self.thread) {
            (Some(thread), Some(own)) if thread != own => self
                .agents
                .get(thread)
                .map_or(Owner::Foreign, |task| Owner::Agent(task.clone())),
            _ => Owner::Own,
        }
    }

    /// Shows a message or reasoning item unless the other line that writes it already did.
    fn once(&mut self, id: String, body: EventBody, out: &mut Out) {
        if self.shown.insert(id.clone()) {
            out.push(id, body);
        }
    }

    /// An assistant message, unless it says nothing.
    fn reply(&mut self, id: String, text: String, out: &mut Out) {
        if !text.trim().is_empty() {
            self.once(id, EventBody::Assistant { text }, out);
        }
    }

    /// A tool item: takes the place of the running call it completes, or shows alone. Only an item of this rollout's
    /// own thread (`own`) completes a call: every running call is this thread's, and a subagent's item shows alone.
    fn tool_item(&mut self, item: &Value, id: String, own: bool, out: &mut Out) {
        let (mut tool, command) = typed_tool(item);
        let claimed = if own {
            self.claim(tool.kind, command.as_deref())
        } else {
            None
        };
        let id = claimed.unwrap_or(id);
        if str_at(item, "/type") == Some("CollabAgentToolCall") {
            let spawned = self.spawned(item, &id);
            if spawned && str_at(item, "/tool") == Some("spawn_agent") && tool.status == ToolStatus::Ok {
                // The subagent runs on until its notification.
                tool.status = ToolStatus::Running;
            }
        }
        self.tools.insert(id.clone(), tool.clone());
        out.push(id, EventBody::Tool(tool));
        if let Some(states) = item.get("agents_states").and_then(Value::as_object) {
            for (thread, state) in states {
                if let Some(outcome) = agent_outcome(state) {
                    self.agent_finished(thread, &outcome, false, out);
                }
            }
        }
    }

    /// Records the subagents a `spawn_agent` item started under task tool `id`; whether it names any.
    fn spawned(&mut self, item: &Value, id: &str) -> bool {
        if str_at(item, "/tool") != Some("spawn_agent") {
            return false;
        }
        let threads = strings(item.get("receiver_thread_ids"));
        let agents = item.get("receiver_agents").and_then(Value::as_array);
        let paths = agents
            .into_iter()
            .flatten()
            .filter_map(|agent| non_empty(agent, "/agent_path"));
        for thread in threads.iter().copied().chain(paths) {
            self.agents.insert(thread.to_owned(), id.to_owned());
        }
        !threads.is_empty()
    }

    /// The running call a tool item of `kind` completes: the one running `command`, else the oldest whose script calls
    /// a tool of that kind.
    fn claim(&mut self, kind: ToolKind, command: Option<&str>) -> Option<String> {
        let by_command = command.and_then(|command| {
            self.calls
                .iter()
                .position(|call| !call.claimed && call.command.as_deref() == Some(command))
        });
        let index = by_command.or_else(|| {
            self.calls
                .iter()
                .position(|call| !call.claimed && call.kinds.contains(&kind))
        })?;
        let call = &mut self.calls[index];
        call.claimed = true;
        Some(call.id.clone())
    }

    /// Subagent `thread` reached a final state: its task tool finishes with `outcome`. A notification updates a task
    /// that already finished (a resumed subagent notifies again), and shows alone when its spawn is not in the window;
    /// a state another collab item reports changes only a task still running.
    fn agent_finished(&mut self, thread: &str, outcome: &(ToolStatus, String), notified: bool, out: &mut Out) {
        let id = match self.agents.get(thread) {
            Some(id) => id.clone(),
            None if notified => {
                self.agents.insert(thread.to_owned(), thread.to_owned());
                thread.to_owned()
            }
            None => return,
        };
        let mut tool = self
            .tools
            .get(&id)
            .cloned()
            .unwrap_or_else(|| untitled_tool(ToolKind::Task, ToolStatus::Running));
        if !notified && tool.status != ToolStatus::Running {
            return;
        }
        finish(&mut tool, outcome.0, &outcome.1);
        self.tools.insert(id.clone(), tool.clone());
        out.push(id, EventBody::Tool(tool));
    }

    fn response(&mut self, payload: &Value, out: &mut Out) {
        let id = str_at(payload, "/id").map_or_else(|| out.line_id(), str::to_owned);
        match str_at(payload, "/type").unwrap_or_default() {
            "message" => match str_at(payload, "/role").unwrap_or_default() {
                "assistant" => self.reply(id, texts(payload, "/content", "output_text"), out),
                "user" => self.notification(&texts(payload, "/content", "input_text"), out),
                _ => {}
            },
            "reasoning" => {
                let summary = payload.get("summary").and_then(Value::as_array);
                let titles: Vec<_> = summary
                    .into_iter()
                    .flatten()
                    .filter_map(|s| str_at(s, "/text"))
                    .collect();
                self.once(
                    id,
                    EventBody::Thinking {
                        text: titles.join("\n"),
                    },
                    out,
                );
            }
            "custom_tool_call" | "function_call" => self.call(payload, out),
            "custom_tool_call_output" | "function_call_output" => self.call_output(payload, out),
            kind if IGNORED_RESPONSES.contains(&kind) => {}
            kind => out.push(
                id,
                EventBody::Unknown {
                    entry: format!("response_item:{kind}"),
                },
            ),
        }
    }

    /// A `<subagent_notification>`: the subagent it names reached a final state.
    fn notification(&mut self, text: &str, out: &mut Out) {
        let text = text.trim();
        let Some(body) = tag(text, "subagent_notification").filter(|_| text.starts_with("<subagent_notification>"))
        else {
            return;
        };
        let Ok(note) = serde_json::from_str::<Value>(body) else {
            return;
        };
        let (Some(thread), Some(outcome)) = (str_at(&note, "/agent_path"), note.get("status").and_then(agent_outcome))
        else {
            return;
        };
        self.agent_finished(thread, &outcome, true, out);
    }

    /// A tool call: running until its output, or until a tool item takes its place.
    fn call(&mut self, payload: &Value, out: &mut Out) {
        let Some(id) = str_at(payload, "/call_id") else { return };
        let name = str_at(payload, "/name").unwrap_or_default();
        // A code-mode script or a freeform tool's input; a function call's JSON arguments.
        let script = str_at(payload, "/input")
            .or_else(|| str_at(payload, "/arguments"))
            .unwrap_or_default();
        if name == WAIT
            && let Some(cell) = cell_of(script)
        {
            self.waits.insert(id.to_owned(), self.cells.get(&cell).cloned());
            return;
        }
        let turn = str_at(payload, "/internal_chat_message_metadata_passthrough/turn_id")
            .map(str::to_owned)
            .or_else(|| self.turn.clone());
        if name == REQUEST_USER_INPUT {
            self.ask(id, script, turn, out);
            return;
        }
        let mut names = called_tools(script);
        names.push(name);
        let kinds: Vec<ToolKind> = names.iter().copied().filter_map(tool_kind).collect();
        let command = kinds
            .contains(&ToolKind::Shell)
            .then(|| js_string(script, "cmd").or_else(|| js_string(script, "command")))
            .flatten();
        let kind = kinds.first().copied().unwrap_or(ToolKind::Other);
        let title = call_title(kind, script, command.as_deref());
        let tool = Tool {
            name: name.to_owned(),
            kind,
            title,
            status: ToolStatus::Running,
            output: None,
            diff: None,
            truncated: None,
        };
        self.calls.push(Call {
            id: id.to_owned(),
            turn,
            kinds,
            command,
            claimed: false,
        });
        self.tools.insert(id.to_owned(), tool.clone());
        out.push(id, EventBody::Tool(tool));
    }

    /// A call's output: finishes the call unless a tool item took its place. An output whose call was shown and has
    /// ended makes no event; one whose call is not in the window shows alone, unless it yielded.
    fn call_output(&mut self, payload: &Value, out: &mut Out) {
        let Some(id) = str_at(payload, "/call_id") else { return };
        if let Some(started) = self.waits.remove(id) {
            if let Some(started) = started {
                self.waited(&started, payload, out);
            }
            return;
        }
        let index = self.calls.iter().position(|call| call.id == id);
        if let Some(cell) = yielded(payload) {
            // the script runs on: a tool item or a `wait`'s output finishes the call
            if index.is_some() {
                self.cells.insert(cell, id.to_owned());
                self.printed.insert(id.to_owned(), printed(payload));
            }
            return;
        }
        let call = index.map(|index| self.calls.remove(index));
        if self.questions.contains_key(id) {
            self.answered(id, payload, out);
            return;
        }
        if call.as_ref().is_some_and(|call| call.claimed) || (call.is_none() && self.tools.contains_key(id)) {
            return;
        }
        let exec = self.tools.get(id).is_some_and(|tool| tool.name == "exec");
        let (status, output) = call_result(payload, exec);
        let mut tool = self
            .tools
            .get(id)
            .cloned()
            .unwrap_or_else(|| untitled_tool(ToolKind::Other, status));
        finish(&mut tool, status, &output);
        self.tools.insert(id.to_owned(), tool.clone());
        out.push(id, EventBody::Tool(tool));
    }

    /// The output of a `wait` on the cell call `started` yielded to: once the script ended, the call finishes with it,
    /// unless a tool item took its place or its turn already ended it.
    fn waited(&mut self, started: &str, payload: &Value, out: &mut Out) {
        if yielded(payload).is_some() {
            if let Some(text) = self.printed.get_mut(started)
                && text.len() <= OUTPUT_CAP
            {
                text.push_str(&printed(payload));
            }
            return;
        }
        self.cells.retain(|_, call| call != started);
        let before = self.printed.remove(started).unwrap_or_default();
        let Some(index) = self.calls.iter().position(|call| call.id == started) else {
            return;
        };
        if self.calls.remove(index).claimed {
            return;
        }
        let (status, output) = call_result(payload, true);
        if let Some(tool) = self.tools.get_mut(started) {
            finish(tool, status, &(before + &output));
            out.push(started, EventBody::Tool(tool.clone()));
        }
    }

    /// A `request_user_input` call: an open question until its output.
    fn ask(&mut self, id: &str, arguments: &str, turn: Option<String>, out: &mut Out) {
        let input: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
        let asked = question(&input);
        let items = input.get("questions").and_then(Value::as_array);
        let ids = items
            .into_iter()
            .flatten()
            .map(|item| str_at(item, "/id").unwrap_or_default().to_owned())
            .collect();
        self.calls.push(Call {
            id: id.to_owned(),
            turn,
            kinds: Vec::new(),
            command: None,
            claimed: false,
        });
        self.questions.insert(id.to_owned(), (asked.clone(), ids));
        out.push(id, EventBody::Question(asked));
    }

    /// A question's output: the answers it got, one line per question, or failed with what Codex wrote instead. An
    /// output after the question ended (its turn did) makes no event.
    fn answered(&mut self, id: &str, payload: &Value, out: &mut Out) {
        let Some((asked, ids)) = self.questions.get_mut(id) else {
            return;
        };
        if asked.status != ToolStatus::Running {
            return;
        }
        let text = match payload.get("output") {
            Some(Value::String(text)) => text.clone(),
            Some(output) => strings_at(output, "/text").concat(),
            None => String::new(),
        };
        let parsed = serde_json::from_str::<Value>(&text).ok();
        let (status, output) = match parsed.as_ref().and_then(|value| value.get("answers")?.as_object()) {
            Some(answers) => (ToolStatus::Ok, answer_lines(asked, ids, answers)),
            None => (ToolStatus::Error, text),
        };
        asked.status = status;
        asked.output = Some(cap(&output, OUTPUT_CAP).0);
        out.push(id, EventBody::Question(asked.clone()));
    }
}

/// The answers a question call got, one line per question in the order asked (`Which size? → Large`), then any its
/// questions don't name.
fn answer_lines(asked: &Question, ids: &[String], answers: &serde_json::Map<String, Value>) -> String {
    let named = asked
        .questions
        .iter()
        .zip(ids)
        .filter_map(|(item, id)| Some(answer_line(&item.question, answers.get(id)?)));
    let others = answers
        .iter()
        .filter(|(id, _)| !ids.contains(id))
        .map(|(id, answer)| answer_line(id, answer));
    named.chain(others).collect::<Vec<_>>().join("\n")
}

/// One question's answer: `question → pick`, then `(note: …)` for a note typed on it.
fn answer_line(question: &str, answer: &Value) -> String {
    let given = strings(answer.get("answers"));
    let (notes, picks): (Vec<&str>, Vec<&str>) = given.iter().partition(|given| given.starts_with(USER_NOTE));
    let mut line = format!("{question} → {}", picks.join(", "));
    for note in notes {
        line.push_str(" (note: ");
        line.push_str(&note[USER_NOTE.len()..]);
        line.push(')');
    }
    line
}

/// The cell a yielded script runs on, from its output's first text: `Script running with cell ID 1…`.
fn yielded(payload: &Value) -> Option<String> {
    let output = payload.get("output")?;
    let head = match output {
        Value::String(text) => text.as_str(),
        _ => strings_at(output, "/text").into_iter().next()?,
    };
    let cell: String = head
        .strip_prefix(SCRIPT_RUNNING)?
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    (!cell.is_empty()).then_some(cell)
}

/// What a yielded script printed since its last yield: what follows its first text's `Output:` line, then the rest.
fn printed(payload: &Value) -> String {
    let blocks = match payload.get("output") {
        Some(Value::String(text)) => vec![text.as_str()],
        Some(output) => strings_at(output, "/text"),
        None => Vec::new(),
    };
    let Some((head, rest)) = blocks.split_first() else {
        return String::new();
    };
    let after = head.split_once("Output:\n").map_or("", |(_, after)| after);
    std::iter::once(after).chain(rest.iter().copied()).collect()
}

/// The cell a `wait` call's JSON arguments name (`{"cell_id":"1",…}`).
fn cell_of(arguments: &str) -> Option<String> {
    let arguments: Value = serde_json::from_str(arguments).ok()?;
    match arguments.get("cell_id")? {
        Value::String(cell) => Some(cell.clone()),
        Value::Number(cell) => Some(cell.to_string()),
        _ => None,
    }
}

fn announce(conversation: &Conversation, out: &mut Out) {
    out.push(
        CONVERSATION_ID,
        EventBody::Conversation {
            agent: AgentKind::Codex,
            conversation_id: conversation.id.clone(),
            cwd: conversation.cwd.clone(),
            model: conversation.model.clone(),
        },
    );
}

/// A `UserMessage` item: the prompt's text and how many images it carried. Its skill mentions are already in the
/// text (`$name`).
fn user_message(item: &Value, id: String, out: &mut Out) {
    let blocks = item.get("content").and_then(Value::as_array);
    let images = blocks
        .into_iter()
        .flatten()
        .filter(|block| str_at(block, "/type").is_some_and(|kind| kind.to_ascii_lowercase().contains("image")))
        .count();
    let text = texts(item, "/content", "text");
    if text.is_empty() && images == 0 {
        return;
    }
    let images = u32::try_from(images).unwrap_or(u32::MAX);
    out.push(
        id,
        EventBody::User {
            text,
            images: (images > 0).then_some(images),
        },
    );
}

/// The `text` of the blocks of type `kind` in the array at `pointer`, joined by newlines.
fn texts(value: &Value, pointer: &str, kind: &str) -> String {
    let blocks = value.pointer(pointer).and_then(Value::as_array);
    blocks
        .into_iter()
        .flatten()
        .filter(|block| str_at(block, "/type") == Some(kind))
        .filter_map(|block| str_at(block, "/text"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The strings of an array.
fn strings(value: Option<&Value>) -> Vec<&str> {
    let items = value.and_then(Value::as_array);
    items.into_iter().flatten().filter_map(Value::as_str).collect()
}

/// What a tool does, from the name a call runs it by (`exec_command`, `multi_agent_v1__spawn_agent`, …).
fn tool_kind(full: &str) -> Option<ToolKind> {
    let name = full.rsplit("__").next().unwrap_or(full);
    match name {
        "exec_command" | "write_stdin" | "shell" | "shell_command" | "local_shell" => Some(ToolKind::Shell),
        "apply_patch" => Some(ToolKind::Edit),
        "spawn_agent" | "wait" | "wait_agent" | "send_input" | "resume_agent" | "close_agent" => Some(ToolKind::Task),
        _ if full.starts_with("web") || name.starts_with("web") || name.contains("web_search") => Some(ToolKind::Web),
        _ => None,
    }
}

/// The tools a code-mode script calls, `tools.<name>(`, in order.
fn called_tools(script: &str) -> Vec<&str> {
    script
        .split("tools.")
        .skip(1)
        .filter_map(|rest| {
            let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
            (end > 0 && rest[end..].starts_with('(')).then(|| &rest[..end])
        })
        .collect()
}

/// A running call's one-line title: the command, the patched file, the URL or query, the subagent's task, or the
/// script's first line.
fn call_title(kind: ToolKind, script: &str, command: Option<&str>) -> String {
    let found = match kind {
        ToolKind::Shell => command.map(str::to_owned),
        ToolKind::Edit => patched_file(script),
        ToolKind::Web => ["query", "q", "url", "ref_id"]
            .iter()
            .find_map(|key| js_string(script, key)),
        ToolKind::Task => ["message", "prompt"].iter().find_map(|key| js_string(script, key)),
        _ => None,
    };
    let fallback = || {
        script
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .to_owned()
    };
    title_line(&found.unwrap_or_else(fallback))
}

/// The file an `apply_patch` script's first `*** Update File:` (or `Add`, `Delete`) line names.
fn patched_file(script: &str) -> Option<String> {
    ["*** Update File: ", "*** Add File: ", "*** Delete File: "]
        .iter()
        .find_map(|marker| {
            let rest = &script[script.find(marker)? + marker.len()..];
            let end = rest.find(['\\', '\n', '"']).unwrap_or(rest.len());
            Some(rest[..end].trim().to_owned())
        })
}

/// The double-quoted string literal a script (or JSON) gives `key`: `key: "…"` or `"key": "…"`, decoded.
fn js_string(script: &str, key: &str) -> Option<String> {
    let mut from = 0;
    while let Some(found) = script[from..].find(key) {
        let start = from + found;
        from = start + key.len();
        let before = script[..start].chars().next_back();
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let rest = &script[from..];
        let rest = rest.strip_prefix('"').unwrap_or(rest).trim_start();
        if let Some(value) = rest.strip_prefix(':').map(str::trim_start).and_then(string_literal) {
            return Some(value);
        }
    }
    None
}

/// The double-quoted string literal `text` starts with, decoded.
fn string_literal(text: &str) -> Option<String> {
    if !text.starts_with('"') {
        return None;
    }
    let mut escaped = false;
    for (index, c) in text.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return serde_json::from_str(&text[..=index]).ok();
        }
    }
    None
}

/// A call output's status and text. `exec` writes `Script completed`, its wall time and `Output:` before the
/// script's output when it ran to its end, and a bare string (`aborted by user after 2.4s`) when it did not. Another
/// call's output is its text, failed when it says `aborted…` or `success: false`.
fn call_result(payload: &Value, exec: bool) -> (ToolStatus, String) {
    let output = payload.get("output").unwrap_or(&Value::Null);
    match output {
        Value::Array(_) => {
            let blocks = strings_at(output, "/text");
            let completed = blocks.first().is_some_and(|head| head.starts_with(SCRIPT_COMPLETED));
            if completed {
                (ToolStatus::Ok, blocks[1..].concat())
            } else {
                (ToolStatus::Error, blocks.concat())
            }
        }
        Value::String(text) => {
            let failed = exec || text.starts_with("aborted");
            let status = if failed { ToolStatus::Error } else { ToolStatus::Ok };
            (status, text.clone())
        }
        Value::Object(_) => {
            let failed = output.get("success").and_then(Value::as_bool) == Some(false);
            let status = if failed { ToolStatus::Error } else { ToolStatus::Ok };
            (status, str_at(output, "/content").unwrap_or_default().to_owned())
        }
        _ => (ToolStatus::Ok, String::new()),
    }
}

/// The string at `pointer` in each element of an array.
fn strings_at<'a>(array: &'a Value, pointer: &str) -> Vec<&'a str> {
    let items = array.as_array();
    items
        .into_iter()
        .flatten()
        .filter_map(|item| str_at(item, pointer))
        .collect()
}

/// A tool item's status: `completed` is ok (for a command, with exit code 0), `in_progress` running, anything else
/// failed (`failed`, `declined`).
fn item_status(item: &Value) -> ToolStatus {
    match str_at(item, "/status") {
        None | Some("completed") => {
            let exit = item.get("exit_code").and_then(Value::as_i64).unwrap_or(0);
            if exit == 0 { ToolStatus::Ok } else { ToolStatus::Error }
        }
        Some("in_progress" | "inProgress" | "running") => ToolStatus::Running,
        Some(_) => ToolStatus::Error,
    }
}

/// The tool a tool item shows, and the shell command it ran, for matching it with its call.
fn typed_tool(item: &Value) -> (Tool, Option<String>) {
    let status = item_status(item);
    let (name, kind, title, output, command) = match str_at(item, "/type").unwrap_or_default() {
        "CommandExecution" => {
            let command = command_script(item);
            let output = str_at(item, "/aggregated_output").unwrap_or_default().to_owned();
            (
                "exec_command".to_owned(),
                ToolKind::Shell,
                command.clone(),
                output,
                Some(command),
            )
        }
        "FileChange" => {
            let paths: Vec<&str> = changes(item).map(|(path, _)| path).collect();
            let output = [str_at(item, "/stdout"), str_at(item, "/stderr")]
                .into_iter()
                .flatten()
                .collect::<String>();
            ("apply_patch".to_owned(), ToolKind::Edit, paths.join(", "), output, None)
        }
        "Extension" => extension(item),
        _ => collab(item),
    };
    let mut tool = Tool {
        name,
        kind,
        title: title_line(&title),
        status: ToolStatus::Running,
        output: None,
        diff: None,
        truncated: None,
    };
    if kind == ToolKind::Edit {
        let (diff, cut) = cap(
            &changes(item)
                .map(|(path, change)| change_diff(path, change))
                .collect::<String>(),
            DIFF_CAP,
        );
        tool.diff = (!diff.is_empty()).then_some(diff);
        tool.truncated = cut.then_some(true);
    }
    if status == ToolStatus::Running {
        let (output, cut) = cap(&output, OUTPUT_CAP);
        tool.output = (!output.is_empty()).then_some(output);
        if cut {
            tool.truncated = Some(true);
        }
    } else {
        finish(&mut tool, status, &output);
    }
    (tool, command)
}

/// The fields a tool item's other kinds show: name, kind, title, output and no command.
type Shown = (String, ToolKind, String, String, Option<String>);

/// An `Extension` item: a web search (`kind: web.search`) or another extension's call.
fn extension(item: &Value) -> Shown {
    let name = str_at(item, "/kind").unwrap_or_default();
    let kind = if name.starts_with("web.") {
        ToolKind::Web
    } else {
        ToolKind::Other
    };
    let title = ["/query", "/action/url", "/action/query"]
        .iter()
        .find_map(|pointer| non_empty(item, pointer))
        .unwrap_or(name);
    let results = item.get("results").and_then(Value::as_array);
    let output = results
        .into_iter()
        .flatten()
        .filter_map(|result| {
            let title = str_at(result, "/title")?;
            Some(match str_at(result, "/url") {
                Some(url) => format!("{title} ({url})\n"),
                None => format!("{title}\n"),
            })
        })
        .collect();
    (name.to_owned(), kind, title.to_owned(), output, None)
}

/// A `CollabAgentToolCall` item: a subagent spawned, waited for, messaged or closed.
fn collab(item: &Value) -> Shown {
    let name = str_at(item, "/tool").unwrap_or_default();
    let agents = item.get("receiver_agents").and_then(Value::as_array);
    let nicknames: Vec<&str> = agents
        .into_iter()
        .flatten()
        .filter_map(|agent| str_at(agent, "/agent_nickname"))
        .collect();
    let title = non_empty(item, "/prompt").map_or_else(
        || {
            if nicknames.is_empty() {
                name.to_owned()
            } else {
                nicknames.join(", ")
            }
        },
        str::to_owned,
    );
    (name.to_owned(), ToolKind::Task, title, String::new(), None)
}

/// The shell script a `CommandExecution` ran: the script of `bash -lc SCRIPT`, else the command's words.
fn command_script(item: &Value) -> String {
    match item.get("command") {
        Some(Value::String(command)) => command.clone(),
        Some(Value::Array(words)) => {
            let words: Vec<&str> = words.iter().filter_map(Value::as_str).collect();
            match words.as_slice() {
                [_, flag, script] if flag.starts_with('-') && flag.ends_with('c') => (*script).to_owned(),
                words => words.join(" "),
            }
        }
        _ => String::new(),
    }
}

/// A `FileChange` item's changes, by path.
fn changes(item: &Value) -> impl Iterator<Item = (&str, &Value)> {
    let changes = item.get("changes").and_then(Value::as_object);
    changes
        .into_iter()
        .flatten()
        .map(|(path, change)| (path.as_str(), change))
}

/// One change's unified diff: an update's own, with its file header; an added or deleted file's whole content.
fn change_diff(path: &str, change: &Value) -> String {
    let content = str_at(change, "/content").unwrap_or_default();
    match str_at(change, "/type").unwrap_or_default() {
        "update" => {
            let to = non_empty(change, "/move_path").unwrap_or(path);
            let hunks = str_at(change, "/unified_diff").unwrap_or_default();
            format!("--- {path}\n+++ {to}\n{hunks}")
        }
        "add" => unified(path, "", content),
        "delete" => unified(path, content, ""),
        _ => String::new(),
    }
}

/// A subagent's final state (`{"completed": …}`, `{"errored": …}`, `shutdown`, `not_found`) as its task's status and
/// output; `None` while it is still starting or running.
fn agent_outcome(state: &Value) -> Option<(ToolStatus, String)> {
    match state {
        Value::String(state) => match state.as_str() {
            "shutdown" => Some((ToolStatus::Error, "Agent was shut down".to_owned())),
            "not_found" => Some((ToolStatus::Error, "Agent not found".to_owned())),
            _ => None,
        },
        Value::Object(map) => {
            let (key, value) = map.iter().next()?;
            let text = value.as_str().unwrap_or_default().to_owned();
            match key.as_str() {
                "completed" => Some((ToolStatus::Ok, text)),
                "errored" => Some((ToolStatus::Error, text)),
                _ => None,
            }
        }
        _ => None,
    }
}
