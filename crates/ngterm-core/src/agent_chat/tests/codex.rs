//! The Codex parser on Task 1's recorded rollouts (golden NDJSON next to each fixture) and on hand-made lines for the
//! edge cases.

use std::path::PathBuf;

use serde_json::json;

use super::support::{self, last, parse_bytes, tool};
use crate::agent_chat::event::{MALFORMED, OUTPUT_CAP};
use crate::agent_chat::{
    AgentKind, ChatEvent, CodexParser, EventBody, NoticeKind, Question, QuestionItem, QuestionOption, ToolKind,
    ToolStatus,
};

const MAIN: &str = "rollout-2026-11-03T23-01-46-01a1a300-e2e3-76a1-abcf-961cbbb2eb3c";
const MAIN_THREAD: &str = "01a1a300-e2e3-76a1-abcf-961cbbb2eb3c";
const SUBAGENT: &str = "rollout-2026-11-03T23-02-28-01a1a301-8737-70e2-bc57-72fa1f1f57c7";
const SUBAGENT_THREAD: &str = "01a1a301-8737-70e2-bc57-72fa1f1f57c7";
const CLEARED: &str = "rollout-2026-11-03T23-05-02-01a1a303-e11f-78a0-a04d-9278c0505c8a";
const AUTO_REVIEW: &str = "rollout-2026-11-03T23-06-17-01a1a305-0669-7522-9a3a-98fa4f56da28";
const GUARDIAN: &str = "rollout-2026-11-03T23-06-17-01a1a305-068c-7b83-a104-15cbf0d10ef0";
/// The main rollout's `exec` call that spawned the subagent.
const SPAWN_CALL: &str = "call_9f3165ba2177432893f811ac93280ff7";

fn sessions_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agent_chat/codex/sessions/2026/11/03")
}

fn fixture(stem: &str) -> Vec<u8> {
    std::fs::read(sessions_dir().join(format!("{stem}.jsonl"))).unwrap()
}

fn parse_fixture(parser: &mut CodexParser, stem: &str) -> Vec<ChatEvent> {
    parse_bytes(parser, &fixture(stem))
}

fn main_events() -> Vec<ChatEvent> {
    parse_fixture(&mut CodexParser::new(), MAIN)
}

/// The subagent's rollout, read under the task tool the main rollout names for its thread.
fn subagent_events() -> Vec<ChatEvent> {
    let mut main = CodexParser::new();
    parse_fixture(&mut main, MAIN);
    let parent = main.task_for_thread(SUBAGENT_THREAD).unwrap();
    parse_fixture(&mut CodexParser::subagent(parent), SUBAGENT)
}

fn golden_case(stem: &str, events: &[ChatEvent]) {
    support::golden_case(&sessions_dir().join(format!("{stem}.golden.ndjson")), events);
}

#[test]
fn the_main_rollout_matches_its_golden() {
    golden_case(MAIN, &main_events());
}

#[test]
fn the_subagent_rollout_matches_its_golden() {
    golden_case(SUBAGENT, &subagent_events());
}

#[test]
fn the_cleared_rollout_matches_its_golden() {
    golden_case(CLEARED, &parse_fixture(&mut CodexParser::new(), CLEARED));
}

#[test]
fn the_auto_review_rollout_matches_its_golden() {
    golden_case(AUTO_REVIEW, &parse_fixture(&mut CodexParser::new(), AUTO_REVIEW));
}

#[test]
fn the_guardian_rollout_matches_its_golden() {
    golden_case(GUARDIAN, &parse_fixture(&mut CodexParser::new(), GUARDIAN));
}

/// Codex 0.167.2's rollout: an escalated command that waited for its approval, then two `request_user_input` calls.
fn questions_path(extension: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "tests/fixtures/agent_chat/codex/sessions/2026/11/12/{QUESTIONS}.{extension}"
    ))
}

const QUESTIONS: &str = "rollout-2026-11-12T15-00-06-01a1cfa1-2430-7c72-b0a1-d372f8a3b729";
/// The escalated command's `exec` call, which yielded while its approval was up.
const ESCALATED_CALL: &str = "call_2243cc81f50c4909b93bcca5308807c8";
/// The `wait` on the escalated command's cell.
const WAIT_CALL: &str = "call_e537b989df864723917a50fc751d41bb";
const ONE_QUESTION: &str = "call_8b4c1d6affc34a70b4c4775cc27d69ec";
const TWO_QUESTIONS: &str = "call_5d0478dc175b4264a25ef38086dd97ce";

fn questions_events() -> Vec<ChatEvent> {
    parse_bytes(
        &mut CodexParser::new(),
        &std::fs::read(questions_path("jsonl")).unwrap(),
    )
}

#[test]
fn the_questions_rollout_matches_its_golden() {
    support::golden_case(&questions_path("golden.ndjson"), &questions_events());
}

fn question(event: &ChatEvent) -> &Question {
    match &event.body {
        EventBody::Question(question) => question,
        other => panic!("not a question: {other:?}"),
    }
}

#[test]
fn an_escalated_command_is_one_row_its_item_completes() {
    let events = questions_events();
    assert_eq!(
        history(&events, ESCALATED_CALL),
        [(ToolStatus::Running, None), (ToolStatus::Ok, Some("HTTP/2 200 \r\n"))],
        "the yielded script runs on until its CommandExecution"
    );
    assert!(
        events
            .iter()
            .all(|event| !event.id.starts_with("exec-") && event.id != WAIT_CALL),
        "no second row for the command, none for the wait"
    );
}

#[test]
fn request_user_input_is_a_question_its_answers_complete() {
    let events = questions_events();
    let asked: Vec<&ChatEvent> = events.iter().filter(|event| event.id == ONE_QUESTION).collect();
    assert_eq!(asked.len(), 2);
    let open = question(asked[0]);
    assert_eq!((open.status, open.output.as_deref()), (ToolStatus::Running, None));
    assert_eq!(
        open.questions,
        [QuestionItem {
            question: "Which color should the button be?".into(),
            header: Some("Button color".into()),
            multi_select: false,
            options: ["Red", "Green", "Blue"]
                .map(|color| QuestionOption {
                    label: color.into(),
                    description: Some(format!("Use {} for the button.", color.to_lowercase())),
                })
                .to_vec(),
        }]
    );
    let answered = question(asked[1]);
    assert_eq!(
        (answered.status, answered.output.as_deref()),
        (ToolStatus::Ok, Some("Which color should the button be? → Green"))
    );
    let two = question(last(&events, TWO_QUESTIONS));
    assert_eq!(
        two.questions.iter().map(|item| item.options.len()).collect::<Vec<_>>(),
        [3, 2],
        "Codex's own None of the above is no option"
    );
    assert_eq!(
        (two.status, two.output.as_deref()),
        (
            ToolStatus::Ok,
            Some("Which size? → Large\nWhich shape? → None of the above (note: hexagon please)")
        ),
        "in the order asked, the note after its pick"
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(&event.body, EventBody::Assistant { text } if text.is_empty())),
        "the empty final answers after the questions show nothing"
    );
}

fn yielded_output(call: &str) -> String {
    line(
        "response_item",
        &json!({"type": "custom_tool_call_output", "call_id": call,
                "output": "Script running with cell ID 7\nWall time 10.0 seconds\nOutput:\n"}),
    )
}

fn wait_call(call: &str, cell_id: &serde_json::Value) -> String {
    let arguments = json!({"cell_id": cell_id, "yield_time_ms": 1000}).to_string();
    line(
        "response_item",
        &json!({"type": "function_call", "call_id": call, "name": "wait", "arguments": arguments}),
    )
}

fn wait_output(call: &str, blocks: &[&str]) -> String {
    let output: Vec<serde_json::Value> = blocks
        .iter()
        .map(|text| json!({"type": "input_text", "text": text}))
        .collect();
    line(
        "response_item",
        &json!({"type": "function_call_output", "call_id": call, "output": output}),
    )
}

#[test]
fn a_yielded_script_runs_on_until_a_wait_sees_it_end() {
    let events = parse(&[
        turn_started("t1"),
        exec_call("c1", "t1", "text(await tools.web_search({query:\"rust\"}));"),
        yielded_output("c1"),
        wait_call("w1", &json!("7")),
        wait_output(
            "w1",
            &["Script running with cell ID 7\nWall time 1.0 seconds\nOutput:\n"],
        ),
        wait_call("w2", &json!(7)),
        wait_output("w2", &["Script completed\nWall time 0.1 seconds\nOutput:\n", "results"]),
    ]);
    assert_eq!(
        history(&events, "c1"),
        [(ToolStatus::Running, None), (ToolStatus::Ok, Some("results"))],
        "a wait that sees it still running changes nothing"
    );
    assert_eq!(
        events.iter().map(|event| event.id.as_str()).collect::<Vec<_>>(),
        ["c1", "c1"],
        "the waits make no row"
    );
}

#[test]
fn a_yielded_script_its_turn_ends_is_an_error_and_a_late_wait_changes_nothing() {
    let events = parse(&[
        turn_started("t1"),
        exec_call("c1", "t1", &shell("sleep 600")),
        yielded_output("c1"),
        wait_call("w1", &json!("7")),
        event_line(&json!({"type": "turn_aborted", "turn_id": "t1", "reason": "interrupted"})),
        wait_output("w1", &["Script completed\nWall time 0.1 seconds\nOutput:\n", "late"]),
        // a wait on a cell this window never saw yield
        wait_call("w2", &json!("9")),
        wait_output("w2", &["Script completed\nWall time 0.1 seconds\nOutput:\n", "other"]),
    ]);
    assert_eq!(
        history(&events, "c1"),
        [(ToolStatus::Running, None), (ToolStatus::Error, Some("Interrupted"))]
    );
    assert!(events.iter().all(|event| !event.id.starts_with('w')));
}

fn ask_call(call: &str, turn: &str) -> String {
    let arguments = json!({"questions": [{"id": "size", "header": "Size", "question": "Which size?",
                                          "options": [{"label": "Small", "description": "A small one."}]}]});
    line(
        "response_item",
        &json!({"type": "function_call", "call_id": call, "name": "request_user_input",
                "arguments": arguments.to_string(),
                "internal_chat_message_metadata_passthrough": {"turn_id": turn}}),
    )
}

fn ask_output(call: &str, output: &str) -> String {
    line(
        "response_item",
        &json!({"type": "function_call_output", "call_id": call, "output": output}),
    )
}

fn question_history(events: &[ChatEvent], id: &str) -> Vec<(ToolStatus, Option<String>)> {
    events
        .iter()
        .filter(|event| event.id == id)
        .map(|event| (question(event).status, question(event).output.clone()))
        .collect()
}

#[test]
fn a_question_its_turn_ends_is_an_error_and_a_late_answer_changes_nothing() {
    let events = parse(&[
        turn_started("t1"),
        ask_call("q1", "t1"),
        event_line(&json!({"type": "turn_aborted", "turn_id": "t1", "reason": "interrupted"})),
        ask_output("q1", r#"{"answers":{"size":{"answers":["Small"]}}}"#),
    ]);
    assert_eq!(
        question_history(&events, "q1"),
        [
            (ToolStatus::Running, None),
            (ToolStatus::Error, Some("Interrupted".into()))
        ]
    );
}

#[test]
fn a_question_output_with_no_answers_fails_with_its_text_and_unasked_answers_still_show() {
    let events = parse(&[
        turn_started("t1"),
        ask_call("q1", "t1"),
        ask_output("q1", "request_user_input was cancelled"),
        ask_call("q2", "t1"),
        ask_output(
            "q2",
            r#"{"answers":{"size":{"answers":["Small","user_note: tiny"]},"extra":{"answers":["x"]}}}"#,
        ),
    ]);
    assert_eq!(
        question_history(&events, "q1").last().unwrap(),
        &(ToolStatus::Error, Some("request_user_input was cancelled".into()))
    );
    assert_eq!(
        question_history(&events, "q2").last().unwrap(),
        &(
            ToolStatus::Ok,
            Some("Which size? → Small (note: tiny)\nextra → x".into())
        )
    );
}

fn users(events: &[ChatEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::User { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn notices(events: &[ChatEvent], wanted: NoticeKind) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::Notice { kind, text, .. } if *kind == wanted => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_conversation_comes_first_and_its_model_follows() {
    let events = main_events();
    let EventBody::Conversation {
        agent,
        conversation_id,
        cwd,
        model,
    } = &events[0].body
    else {
        panic!("first event: {:?}", events[0]);
    };
    assert_eq!((*agent, conversation_id.as_str()), (AgentKind::Codex, MAIN_THREAD));
    assert_eq!((cwd.as_deref(), model.as_deref()), (Some("/home/user/project"), None));
    let EventBody::Conversation { model, .. } = &last(&events, "conversation").body else {
        panic!()
    };
    assert_eq!(model.as_deref(), Some("gpt-5.6-luna"), "from the first turn_context");
}

#[test]
fn messages_written_by_both_lines_show_once() {
    let events = main_events();
    let mut ids: Vec<&str> = events
        .iter()
        .filter(|event| matches!(event.body, EventBody::Assistant { .. } | EventBody::Thinking { .. }))
        .map(|event| event.id.as_str())
        .collect();
    let count = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), count, "each message and reasoning id once");
    assert_eq!(count, 21, "10 agent messages and 11 reasoning items");
    let prompts = users(&events);
    assert_eq!(
        prompts.len(),
        8,
        "one per UserMessage item, none from the response_item copies"
    );
    assert!(
        prompts.iter().all(|text| !text.starts_with('<')),
        "no Codex context: {prompts:#?}"
    );
    assert!(prompts.contains(&"$greet-check please"), "a skill shows as its mention");
}

#[test]
fn a_response_item_message_alone_still_shows_once() {
    let message = json!({"type": "message", "role": "assistant", "id": "msg_1",
                         "content": [{"type": "output_text", "text": "from the model"}]});
    let item = json!({"type": "AgentMessage", "id": "msg_1", "content": [{"type": "Text", "text": "from the model"}]});
    let events = parse(&[line("response_item", &message), item_line(None, "t", &item)]);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].body,
        EventBody::Assistant {
            text: "from the model".into()
        }
    );
}

#[test]
fn tool_items_take_their_calls_place() {
    let events = main_events();
    let ls = tool(last(&events, "call_b3183fcc09224f4cabc0406696fe078c"));
    assert_eq!(
        (ls.kind, ls.title.as_str(), ls.status),
        (ToolKind::Shell, "ls -la", ToolStatus::Ok)
    );
    assert!(ls.output.as_deref().unwrap().contains("hello.py"));
    let web = tool(last(&events, "call_e5032d816e6546bd998bf9f5ffd56a8f"));
    assert_eq!(
        (web.kind, web.title.as_str(), web.output.as_deref()),
        (
            ToolKind::Web,
            "https://example.com",
            Some("Example Domain (https://example.com/)\n")
        )
    );
    let edit = tool(last(&events, "call_8ca3d9d159c9499cb9cc06df9ea42440"));
    assert_eq!((edit.kind, edit.status), (ToolKind::Edit, ToolStatus::Ok));
    let diff = edit.diff.as_deref().unwrap();
    assert!(diff.starts_with("--- /home/user/project/hello.py\n+++ /home/user/project/hello.py\n@@ -2,3 +2,3 @@\n"));
    assert!(diff.contains("+    return \"Hi, \" + name + \"!\"\n"));
    let script = tool(last(&events, "call_d140dedb1ef04d3091489331f6da5614"));
    assert_eq!(
        (script.name.as_str(), script.kind, script.status),
        ("exec", ToolKind::Other, ToolStatus::Ok),
        "a script no tool item claims finishes with its own output"
    );
    assert!(
        script
            .output
            .as_deref()
            .unwrap()
            .starts_with("Warning: truncated output")
    );
    let ids: Vec<&str> = events
        .iter()
        .filter_map(|event| event.id.strip_prefix("exec-"))
        .collect();
    assert_eq!(ids, Vec::<&str>::new(), "no tool item shows apart from its call");
}

#[test]
fn a_spawned_subagent_runs_until_its_notification() {
    let events = main_events();
    let states: Vec<ToolStatus> = events
        .iter()
        .filter(|event| event.id == SPAWN_CALL)
        .map(|event| tool(event).status)
        .collect();
    assert_eq!(states, [ToolStatus::Running, ToolStatus::Running, ToolStatus::Ok]);
    let done = tool(last(&events, SPAWN_CALL));
    assert_eq!(
        (done.kind, done.output.as_deref()),
        (ToolKind::Task, Some("`notes.txt` contains **3 lines**."))
    );
}

#[test]
fn a_subagents_rollout_folds_under_its_task_without_the_parents_history() {
    let events = subagent_events();
    assert!(events.iter().all(|event| event.parent.as_deref() == Some(SPAWN_CALL)));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.body, EventBody::Conversation { .. })),
        "a subagent announces no conversation"
    );
    assert_eq!(
        users(&events),
        ["In the project workspace, run `wc -l notes.txt` and report the line count. Do not modify files."],
        "the parent's history the rollout starts with is skipped"
    );
}

#[test]
fn a_denied_call_is_an_error_then_an_interrupt() {
    let events = main_events();
    let denied = tool(last(&events, "call_ef4bdc1e1df44891846190b0cc0abdbc"));
    assert_eq!(
        (denied.title.as_str(), denied.status, denied.output.as_deref()),
        (
            "curl -sI https://example.org .",
            ToolStatus::Error,
            Some("aborted by user after 2.4s")
        )
    );
    assert_eq!(
        notices(&events, NoticeKind::Interrupt),
        ["Turn interrupted by user", "Turn interrupted by user"],
        "the denial, then the reply interrupted with Esc"
    );
}

#[test]
fn compaction_is_one_notice() {
    let events = main_events();
    assert_eq!(notices(&events, NoticeKind::Compaction), ["Conversation compacted"]);
}

#[test]
fn a_multi_line_script_still_titles_its_command() {
    let events = parse_fixture(&mut CodexParser::new(), AUTO_REVIEW);
    let running = tool(&events[5]);
    assert_eq!(
        (running.status, running.title.as_str()),
        (ToolStatus::Running, "curl -sI https://example.com")
    );
}

fn parse(lines: &[String]) -> Vec<ChatEvent> {
    parse_bytes(&mut CodexParser::new(), lines.join("\n").as_bytes())
}

fn line(kind: &str, payload: &serde_json::Value) -> String {
    json!({"type": kind, "payload": payload}).to_string()
}

fn meta(thread: &str) -> String {
    line("session_meta", &json!({"id": thread, "cwd": "/p"}))
}

fn event_line(payload: &serde_json::Value) -> String {
    line("event_msg", payload)
}

fn item_line(thread: Option<&str>, turn: &str, item: &serde_json::Value) -> String {
    event_line(&json!({"type": "item_completed", "thread_id": thread, "turn_id": turn, "item": item}))
}

fn turn_started(turn: &str) -> String {
    event_line(&json!({"type": "task_started", "turn_id": turn}))
}

fn exec_call(call: &str, turn: &str, script: &str) -> String {
    line(
        "response_item",
        &json!({"type": "custom_tool_call", "call_id": call, "name": "exec", "input": script,
                "internal_chat_message_metadata_passthrough": {"turn_id": turn}}),
    )
}

fn exec_output(call: &str, text: &str) -> String {
    line(
        "response_item",
        &json!({"type": "custom_tool_call_output", "call_id": call, "output": [
            {"type": "input_text", "text": "Script completed\nWall time 0.1 seconds\nOutput:\n"},
            {"type": "input_text", "text": text}]}),
    )
}

fn shell(command: &str) -> String {
    format!("const r = await tools.exec_command({{cmd:{command:?}}}); text(r.output);\n")
}

fn command_item(id: &str, command: &str, exit_code: i64) -> serde_json::Value {
    json!({"type": "CommandExecution", "id": id, "command": ["/usr/bin/bash", "-lc", command],
           "status": "completed", "aggregated_output": format!("{command} ran\n"), "exit_code": exit_code})
}

fn spawn_item(thread: &str) -> serde_json::Value {
    json!({"type": "CollabAgentToolCall", "id": "exec-spawn", "tool": "spawn_agent", "status": "completed",
           "receiver_thread_ids": [thread], "prompt": "count lines", "agents_states": {thread: "pending_init"}})
}

fn spawn_call(call: &str, turn: &str) -> String {
    exec_call(
        call,
        turn,
        "text(await tools.multi_agent_v1__spawn_agent({message:\"count lines\"}));",
    )
}

fn notification(thread: &str, status: &serde_json::Value) -> String {
    let note = json!({"agent_path": thread, "status": status});
    line(
        "response_item",
        &json!({"type": "message", "role": "user", "content": [
            {"type": "input_text", "text": format!("<subagent_notification>\n{note}\n</subagent_notification>")}]}),
    )
}

/// The statuses and outputs the events with `id` show, in order.
fn history<'a>(events: &'a [ChatEvent], id: &str) -> Vec<(ToolStatus, Option<&'a str>)> {
    events
        .iter()
        .filter(|event| event.id == id)
        .map(|event| (tool(event).status, tool(event).output.as_deref()))
        .collect()
}

#[test]
fn a_call_with_no_output_yet_is_running() {
    let events = parse(&[turn_started("t1"), exec_call("c1", "t1", &shell("sleep 60"))]);
    let running = tool(&events[0]);
    assert_eq!(
        (
            events[0].id.as_str(),
            running.kind,
            running.title.as_str(),
            running.status
        ),
        ("c1", ToolKind::Shell, "sleep 60", ToolStatus::Running)
    );
}

#[test]
fn an_interrupted_turn_ends_its_running_calls() {
    let events = parse(&[
        turn_started("t1"),
        exec_call("c1", "t1", &shell("sleep 60")),
        event_line(&json!({"type": "turn_aborted", "turn_id": "t1", "reason": "interrupted"})),
        exec_output("c1", "late"),
    ]);
    assert_eq!(
        history(&events, "c1"),
        [(ToolStatus::Running, None), (ToolStatus::Error, Some("Interrupted"))],
        "an output after the turn ended changes nothing"
    );
    assert_eq!(notices(&events, NoticeKind::Interrupt), ["Turn interrupted by user"]);
}

#[test]
fn a_call_left_behind_by_a_crash_ends_when_the_next_turn_starts() {
    let events = parse(&[
        turn_started("t1"),
        exec_call("c1", "t1", &shell("sleep 60")),
        turn_started("t2"),
        exec_call("c2", "t2", &shell("true")),
    ]);
    assert_eq!(
        history(&events, "c1"),
        [
            (ToolStatus::Running, None),
            (ToolStatus::Error, Some("No result was recorded"))
        ]
    );
    assert_eq!(
        history(&events, "c2"),
        [(ToolStatus::Running, None)],
        "the new turn's call runs"
    );
}

#[test]
fn a_completed_turn_ends_only_its_own_calls() {
    let events = parse(&[
        turn_started("t1"),
        exec_call("c1", "t1", &shell("sleep 60")),
        exec_call("c2", "other", &shell("sleep 60")),
        event_line(&json!({"type": "task_complete", "turn_id": "t1"})),
    ]);
    assert_eq!(history(&events, "c1").last().unwrap().0, ToolStatus::Error);
    assert_eq!(history(&events, "c2"), [(ToolStatus::Running, None)]);
}

#[test]
fn a_subagent_outlives_its_turn_and_a_late_spawn_output() {
    let events = parse(&[
        turn_started("t1"),
        spawn_call("c1", "t1"),
        item_line(None, "t1", &spawn_item("th1")),
        event_line(&json!({"type": "turn_aborted", "turn_id": "t1", "reason": "interrupted"})),
        exec_output("c1", "{\"agent_id\":\"th1\"}"),
        notification("th1", &json!({"completed": "3 lines"})),
    ]);
    assert_eq!(
        history(&events, "c1"),
        [
            (ToolStatus::Running, None),
            (ToolStatus::Running, None),
            (ToolStatus::Ok, Some("3 lines"))
        ]
    );
}

#[test]
fn parallel_calls_pair_with_their_items_by_command() {
    let events = parse(&[
        turn_started("t1"),
        exec_call("c1", "t1", &shell("ls")),
        exec_call("c2", "t1", &shell("pwd")),
        item_line(None, "t1", &command_item("exec-b", "pwd", 0)),
        item_line(None, "t1", &command_item("exec-a", "ls", 2)),
        exec_output("c2", "pwd ran\n"),
        exec_output("c1", "ls ran\n"),
    ]);
    assert_eq!(
        history(&events, "c1"),
        [(ToolStatus::Running, None), (ToolStatus::Error, Some("ls ran\n"))],
        "a non-zero exit is an error; the claimed call's own output adds nothing"
    );
    assert_eq!(
        history(&events, "c2"),
        [(ToolStatus::Running, None), (ToolStatus::Ok, Some("pwd ran\n"))]
    );
}

#[test]
fn an_item_no_call_claims_shows_alone() {
    let events = parse(&[
        turn_started("t1"),
        item_line(None, "t1", &command_item("exec-a", "ls", 0)),
    ]);
    assert_eq!(events[0].id, "exec-a");
    assert_eq!(tool(&events[0]).title, "ls");
}

#[test]
fn a_subagents_items_in_the_parent_fold_under_its_task() {
    let message = json!({"type": "AgentMessage", "id": "m1", "content": [{"type": "Text", "text": "child"}]});
    let stranger = json!({"type": "AgentMessage", "id": "m2", "content": [{"type": "Text", "text": "stranger"}]});
    let events = parse(&[
        meta("main"),
        turn_started("t1"),
        spawn_call("c1", "t1"),
        item_line(Some("main"), "t1", &spawn_item("th1")),
        item_line(Some("th1"), "s1", &message),
        item_line(Some("elsewhere"), "s2", &stranger),
    ]);
    assert_eq!(last(&events, "m1").parent.as_deref(), Some("c1"));
    assert!(
        !events.iter().any(|event| event.id == "m2"),
        "another thread's item is not this chat's"
    );
}

#[test]
fn a_subagents_tool_item_in_the_parent_claims_none_of_the_parents_calls() {
    let child_wc = command_item("exec-child-wc", "wc -l notes.txt", 0);
    let child_sleep = command_item("exec-child-sleep", "sleep 5", 0);
    let events = parse(&[
        meta("main"),
        turn_started("t1"),
        spawn_call("c1", "t1"),
        item_line(Some("main"), "t1", &spawn_item("th1")),
        exec_call("c9", "t1", &shell("sleep 5")),
        item_line(Some("th1"), "s1", &child_wc),
        item_line(Some("th1"), "s1", &child_sleep),
        item_line(Some("main"), "t1", &command_item("exec-own", "sleep 5", 0)),
        exec_output("c9", "sleep 5 ran\n"),
    ]);
    for child in ["exec-child-wc", "exec-child-sleep"] {
        let event = last(&events, child);
        assert_eq!(event.parent.as_deref(), Some("c1"), "{child} folds under its task");
        assert_eq!(tool(event).status, ToolStatus::Ok);
    }
    assert!(
        events
            .iter()
            .filter(|event| event.id == "c9")
            .all(|event| event.parent.is_none()),
        "the parent's call stays the parent's"
    );
    assert_eq!(
        history(&events, "c9"),
        [(ToolStatus::Running, None), (ToolStatus::Ok, Some("sleep 5 ran\n"))],
        "the parent's own item claims its call, whose output then adds nothing"
    );
    assert!(
        !events.iter().any(|event| event.id == "exec-own"),
        "the parent's item takes its call's place"
    );
}

#[test]
fn a_running_commands_clipped_output_is_marked_truncated() {
    let mut running = command_item("exec-a", "yes", 0);
    running["status"] = json!("in_progress");
    running["aggregated_output"] = json!("y\n".repeat(OUTPUT_CAP));
    let events = parse(&[turn_started("t1"), item_line(None, "t1", &running)]);
    let shown = tool(&events[0]);
    assert_eq!(shown.status, ToolStatus::Running);
    assert_eq!(shown.output.as_ref().map(String::len), Some(OUTPUT_CAP));
    assert_eq!(shown.truncated, Some(true));
}

#[test]
fn a_notification_updates_its_task_again_and_shows_alone_without_its_spawn() {
    let events = parse(&[
        turn_started("t1"),
        spawn_call("c1", "t1"),
        item_line(None, "t1", &spawn_item("th1")),
        notification("th1", &json!({"completed": "first"})),
        notification("th1", &json!({"errored": "second"})),
        notification("th2", &json!({"completed": "orphan"})),
        notification("th3", &json!("running")),
    ]);
    assert_eq!(
        history(&events, "c1")[2..],
        [(ToolStatus::Ok, Some("first")), (ToolStatus::Error, Some("second"))],
        "a resumed subagent notifies again"
    );
    let orphan = tool(last(&events, "th2"));
    assert_eq!(
        (orphan.name.as_str(), orphan.kind, orphan.status),
        ("", ToolKind::Task, ToolStatus::Ok)
    );
    assert!(
        !events.iter().any(|event| event.id == "th3"),
        "a state that is not final changes nothing"
    );
}

#[test]
fn closing_a_running_subagent_finishes_its_task() {
    let close = json!({"type": "CollabAgentToolCall", "id": "exec-close", "tool": "close_agent",
                       "status": "completed", "receiver_thread_ids": ["th1"], "agents_states": {"th1": "shutdown"}});
    let events = parse(&[
        turn_started("t1"),
        spawn_call("c1", "t1"),
        item_line(None, "t1", &spawn_item("th1")),
        exec_output("c1", "{}"),
        item_line(None, "t1", &close),
        notification("th1", &json!({"completed": "done anyway"})),
    ]);
    assert_eq!(
        history(&events, "c1")[2..],
        [
            (ToolStatus::Error, Some("Agent was shut down")),
            (ToolStatus::Ok, Some("done anyway"))
        ],
        "a notification that still comes updates it"
    );
    let closed = tool(last(&events, "exec-close"));
    assert_eq!((closed.kind, closed.status), (ToolKind::Task, ToolStatus::Ok));
}

#[test]
fn an_output_with_no_call_in_the_window_still_shows() {
    let output = line(
        "response_item",
        &json!({"type": "custom_tool_call_output", "call_id": "gone", "output": "aborted by user after 1s"}),
    );
    let events = parse(&[output]);
    let orphan = tool(&events[0]);
    assert_eq!(
        (orphan.name.as_str(), orphan.title.as_str(), orphan.status),
        ("", "", ToolStatus::Error)
    );
}

#[test]
fn a_function_call_reads_like_a_script() {
    let call = line(
        "response_item",
        &json!({"type": "function_call", "call_id": "f1", "name": "exec_command",
                "arguments": "{\"cmd\":\"ls -la\",\"workdir\":\"/p\"}"}),
    );
    let output = line(
        "response_item",
        &json!({"type": "function_call_output", "call_id": "f1", "output": "listing"}),
    );
    let events = parse(&[call, output]);
    assert_eq!(
        history(&events, "f1"),
        [(ToolStatus::Running, None), (ToolStatus::Ok, Some("listing"))]
    );
    assert_eq!(tool(&events[0]).title, "ls -la");
}

#[test]
fn an_error_is_a_notice() {
    let events = parse(&[event_line(&json!({"type": "error", "message": "stream disconnected"}))]);
    assert_eq!(notices(&events, NoticeKind::ApiError), ["stream disconnected"]);
}

#[test]
fn unknown_types_are_unknown() {
    let events = parse(&[
        line("brand_new", &json!({})),
        event_line(&json!({"type": "brand_new"})),
        line("response_item", &json!({"type": "brand_new", "id": "r"})),
        item_line(None, "t", &json!({"type": "BrandNew", "id": "i"})),
    ]);
    let entries: Vec<_> = events
        .iter()
        .map(|event| match &event.body {
            EventBody::Unknown { entry } => entry.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        entries,
        [
            "brand_new",
            "event_msg:brand_new",
            "response_item:brand_new",
            "item:BrandNew"
        ]
    );
}

#[test]
fn a_truncated_last_line_is_malformed_and_costs_only_itself() {
    let bytes = fixture(MAIN);
    let last_line = bytes[..bytes.len() - 1].iter().rposition(|&b| b == b'\n').unwrap() + 1;
    let cut = &bytes[..last_line + (bytes.len() - last_line) / 2];
    let mut events = parse_bytes(&mut CodexParser::new(), cut);
    let truncated = events.pop().unwrap();
    assert_eq!(
        (truncated.cursor, truncated.body),
        (
            last_line as u64,
            EventBody::Unknown {
                entry: MALFORMED.into()
            }
        )
    );
    assert_eq!(
        events,
        main_events(),
        "the last line (a task_complete) made no event of its own"
    );
}
