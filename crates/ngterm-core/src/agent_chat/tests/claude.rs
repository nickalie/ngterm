//! The Claude parser on Task 1's recorded transcripts (golden NDJSON next to each fixture) and on hand-made lines for
//! the edge cases.

use std::path::PathBuf;

use super::support::{self, last, parse_bytes, tool};
use crate::agent_chat::claude::subagent_parent;
use crate::agent_chat::event::{LINE_CAP, MALFORMED, OUTPUT_CAP, OVERSIZED};
use crate::agent_chat::{
    AgentKind, ChatEvent, ClaudeParser, EventBody, NoticeKind, Question, Tool, ToolKind, ToolStatus, TranscriptParser,
};

const MAIN: &str = "f559ab9f-a5bd-4b2f-91e5-72590d662bf4";
const CLEARED: &str = "2897645c-5554-4c74-ab67-20ec0af2f264";
const WORKER: &str = "17d240d5-8dc2-4980-9a49-0c5015321537";
const SUBAGENT: &str = "f559ab9f-a5bd-4b2f-91e5-72590d662bf4/subagents/agent-a3ef4fec6a51ca74c";
const AGENT_TOOL: &str = "toolu_017cp2PTnCapCdo5DxHUGE69";

fn project_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agent_chat/claude/projects/-home-user-project")
}

fn parse_fixture(parser: &mut dyn TranscriptParser, stem: &str) -> Vec<ChatEvent> {
    parse_bytes(
        parser,
        &std::fs::read(project_dir().join(format!("{stem}.jsonl"))).unwrap(),
    )
}

fn main_events() -> Vec<ChatEvent> {
    parse_fixture(&mut ClaudeParser::new(), MAIN)
}

fn subagent_events() -> Vec<ChatEvent> {
    let meta = std::fs::read(project_dir().join(format!("{SUBAGENT}.meta.json"))).unwrap();
    let parent = subagent_parent(&meta).unwrap();
    parse_fixture(&mut ClaudeParser::subagent(parent), SUBAGENT)
}

fn golden_case(stem: &str, events: &[ChatEvent]) {
    support::golden_case(&project_dir().join(format!("{stem}.golden.ndjson")), events);
}

#[test]
fn the_main_conversation_matches_its_golden() {
    golden_case(MAIN, &main_events());
}

#[test]
fn the_cleared_conversation_matches_its_golden() {
    golden_case(CLEARED, &parse_fixture(&mut ClaudeParser::new(), CLEARED));
}

#[test]
fn the_worker_run_matches_its_golden() {
    golden_case(WORKER, &parse_fixture(&mut ClaudeParser::new(), WORKER));
}

#[test]
fn the_subagent_matches_its_golden() {
    golden_case(SUBAGENT, &subagent_events());
}

fn user_texts(events: &[ChatEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::User { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_conversation_comes_first_and_names_its_model() {
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
    assert_eq!((*agent, conversation_id.as_str()), (AgentKind::Claude, MAIN));
    assert_eq!(cwd.as_deref(), Some("/home/user/project"));
    assert_eq!(model, &None, "the first user line comes before any model");
    let EventBody::Conversation { model, .. } = &last(&events, &events[0].id).body else {
        panic!()
    };
    assert_eq!(model.as_deref(), Some("claude-sonnet-5-5"));
}

#[test]
fn tools_pair_with_their_results_by_id() {
    let events = main_events();
    let ls = tool(last(&events, "toolu_018Guh1UYFQbFS5b5h48k5jn"));
    assert_eq!(
        (ls.kind, ls.title.as_str(), ls.status),
        (ToolKind::Shell, "ls -la", ToolStatus::Ok)
    );
    assert!(ls.output.as_deref().unwrap().contains("hello.py"));
    let read = tool(last(&events, "toolu_012nzFCHZ3KaG2Jgp8byFUic"));
    assert_eq!(
        (read.kind, read.title.as_str()),
        (ToolKind::Read, "/home/user/project/hello.py")
    );
    let fetch = tool(last(&events, "toolu_01PYJZh7HRQtWhbhHUpCUnTR"));
    assert_eq!(
        (fetch.kind, fetch.title.as_str()),
        (ToolKind::Web, "https://example.com")
    );
    let search = tool(last(&events, "toolu_01Sqmnkr6Mi9khzakgzu3DYy"));
    assert_eq!(
        (search.kind, search.name.as_str(), search.status),
        (ToolKind::Other, "ToolSearch", ToolStatus::Ok)
    );
    let first = events
        .iter()
        .find(|event| event.id == "toolu_018Guh1UYFQbFS5b5h48k5jn")
        .unwrap();
    assert_eq!(
        tool(first).status,
        ToolStatus::Running,
        "the call shows before its result"
    );
}

#[test]
fn an_edit_carries_its_diff() {
    let events = main_events();
    let edit = tool(last(&events, "toolu_01JxHgYG7rDbqLRcArHUxmec"));
    assert_eq!(
        (edit.kind, edit.title.as_str(), edit.status),
        (ToolKind::Edit, "/home/user/project/hello.py", ToolStatus::Ok)
    );
    let diff = edit.diff.as_deref().unwrap();
    assert!(
        diff.starts_with("--- /home/user/project/hello.py\n+++ /home/user/project/hello.py\n@@"),
        "{diff}"
    );
    assert!(
        diff.contains("\n-    return \"Hello \" + name\n+    return \"Hello, \" + name + \"!\"\n"),
        "{diff}"
    );
}

#[test]
fn ask_user_question_becomes_a_question() {
    let events = main_events();
    let id = "toolu_015a3PGSqNMwSt5wVKkXKXtq";
    let first = events.iter().find(|event| event.id == id).unwrap();
    let EventBody::Question(Question {
        questions,
        status: ToolStatus::Running,
        output: None,
    }) = &first.body
    else {
        panic!("{first:?}");
    };
    assert_eq!(questions.len(), 2);
    assert_eq!(
        (questions[0].question.as_str(), questions[0].multi_select),
        ("Which greeting style?", false)
    );
    assert_eq!(questions[0].header.as_deref(), Some("Style"));
    let labels: Vec<_> = questions[1]
        .options
        .iter()
        .map(|option| option.label.as_str())
        .collect();
    assert_eq!(labels, ["Spanish", "French", "German"]);
    assert!(questions[1].multi_select);
    assert_eq!(
        questions[1].options[0].description.as_deref(),
        Some("Support Spanish greetings")
    );
    let EventBody::Question(answered) = &last(&events, id).body else {
        panic!()
    };
    assert_eq!(answered.status, ToolStatus::Ok);
    assert!(answered.output.as_deref().unwrap().contains("\"Spanish, German\""));
    assert!(
        !events
            .iter()
            .any(|event| matches!(&event.body, EventBody::Tool(tool) if tool.name == "AskUserQuestion"))
    );
}

#[test]
fn a_denied_call_is_an_error_then_an_interrupt() {
    let events = main_events();
    let at = events
        .iter()
        .rposition(|event| event.id == "toolu_01QKFFDNpAPD8wArBHoPYomY")
        .unwrap();
    let denied = tool(&events[at]);
    assert_eq!(
        (denied.status, denied.title.as_str()),
        (ToolStatus::Error, "touch denied.txt")
    );
    assert!(
        denied
            .output
            .as_deref()
            .unwrap()
            .starts_with("The user doesn't want to proceed")
    );
    let EventBody::Notice {
        kind: NoticeKind::Interrupt,
        text,
        ..
    } = &events[at + 1].body
    else {
        panic!("{:?}", events[at + 1]);
    };
    assert_eq!(text, "Request interrupted by user for tool use");
    let interrupts = events.iter().filter(|event| {
        matches!(
            &event.body,
            EventBody::Notice {
                kind: NoticeKind::Interrupt,
                ..
            }
        )
    });
    assert_eq!(
        interrupts.count(),
        3,
        "denied, dismissed with Esc, and the reply interrupted mid-stream"
    );
}

#[test]
fn a_background_agent_completes_from_its_notification() {
    let events = main_events();
    let calls: Vec<_> = events.iter().filter(|event| event.id == AGENT_TOOL).map(tool).collect();
    assert_eq!(
        calls.len(),
        2,
        "the call, then the notification; the launch receipt changes nothing"
    );
    assert_eq!((calls[0].kind, calls[0].status), (ToolKind::Task, ToolStatus::Running));
    assert_eq!(calls[0].title, "Count lines in notes.txt");
    assert_eq!(
        (calls[1].status, calls[1].title.as_str()),
        (ToolStatus::Ok, "Count lines in notes.txt")
    );
    assert_eq!(
        calls[1].output.as_deref(),
        Some("`wc -l notes.txt` printed a line count of 3.")
    );
    assert!(
        !user_texts(&events)
            .iter()
            .any(|text| text.contains("task-notification"))
    );
}

#[test]
fn compaction_is_one_notice_with_its_summary() {
    let events = main_events();
    let compactions: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::Notice {
                kind: NoticeKind::Compaction,
                text,
                detail,
            } => Some((&event.id, text, detail)),
            _ => None,
        })
        .collect();
    assert_eq!(compactions.len(), 2, "the boundary, then the summary updating it");
    assert_eq!(compactions[0].0, compactions[1].0);
    assert_eq!(compactions[0].1, "Conversation compacted");
    assert!(compactions[1].2.as_deref().unwrap().contains("Summary:"));
}

#[test]
fn bookkeeping_and_local_command_lines_make_no_bubbles() {
    let events = main_events();
    let texts = user_texts(&events);
    assert_eq!(
        texts.iter().filter(|text| text.starts_with("/compact")).count(),
        1,
        "{texts:#?}"
    );
    assert!(
        texts
            .iter()
            .all(|text| !text.contains("<local-command") && !text.contains("<command-name>")),
        "{texts:#?}"
    );
    assert_eq!(
        texts.last(),
        Some(&"Reply with just: resumed"),
        "the resumed turn appends to the same file"
    );
    let thinking = events
        .iter()
        .filter(|event| matches!(&event.body, EventBody::Thinking { text } if text.is_empty()));
    assert_eq!(thinking.count(), 2, "Claude writes thinking empty");
}

#[test]
fn a_cleared_conversation_opens_with_the_command() {
    let events = parse_fixture(&mut ClaudeParser::new(), CLEARED);
    assert_eq!(user_texts(&events), ["/clear", "Reply with just: ok"]);
}

#[test]
fn subagent_events_fold_under_their_task() {
    let events = subagent_events();
    assert!(
        events.iter().all(|event| event.parent.as_deref() == Some(AGENT_TOOL)),
        "{events:#?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.body, EventBody::Conversation { .. }))
    );
    let wc = tool(last(&events, "toolu_015QyE6g8rFUFDzsHowGZHsz"));
    assert_eq!(
        (wc.title.as_str(), wc.output.as_deref()),
        ("wc -l notes.txt", Some("3 notes.txt"))
    );
}

#[test]
fn a_subagents_unreadable_lines_still_fold_under_its_task() {
    let mut oversized = vec![b' '; LINE_CAP + 1];
    oversized[0] = b'{';
    let mut parser = ClaudeParser::subagent(AGENT_TOOL);
    let mut events = parser.parse_line(br#"{"type""#, 0);
    events.extend(parser.parse_line(&oversized, 8));
    assert_eq!(events.len(), 2, "{events:#?}");
    assert!(
        events.iter().all(|event| event.parent.as_deref() == Some(AGENT_TOOL)),
        "{events:#?}"
    );
}

#[test]
fn a_meta_file_without_a_tool_use_id_names_no_parent() {
    assert_eq!(subagent_parent(br#"{"agentType":"general-purpose"}"#), None);
    assert_eq!(subagent_parent(b"not json"), None);
    assert_eq!(
        subagent_parent(br#"{"toolUseId":"toolu_1"}"#).as_deref(),
        Some("toolu_1")
    );
}

fn parse_lines(lines: &[&str]) -> Vec<ChatEvent> {
    parse_bytes(&mut ClaudeParser::new(), lines.join("\n").as_bytes())
}

fn parse_owned(lines: &[String]) -> Vec<ChatEvent> {
    parse_lines(&lines.iter().map(String::as_str).collect::<Vec<_>>())
}

/// A `user` line whose content is `text`.
fn user_line(uuid: &str, text: &str) -> String {
    serde_json::json!({"type": "user", "uuid": uuid, "message": {"content": text}}).to_string()
}

/// The `<command-name>` line Claude Code writes when `/name args` runs.
fn command(uuid: &str, name: &str, args: &str) -> String {
    let text = format!(
        "<command-name>/{name}</command-name>\n            <command-message>{name}</command-message>\n            \
         <command-args>{args}</command-args>"
    );
    user_line(uuid, &text)
}

#[test]
fn a_command_repeats_only_the_prompt_just_before() {
    let reply = r#"{"type":"assistant","uuid":"r","message":{"content":[{"type":"text","text":"done"}]}}"#;
    let events = parse_owned(&[
        user_line("p1", "/compact keep it short  "),
        r#"{"type":"system","subtype":"compact_boundary","uuid":"b"}"#.to_owned(),
        command("c1", "compact", "keep it short"),
        command("c2", "cost", ""),
        command("c3", "cost", ""),
        user_line("p2", "/review"),
        reply.to_owned(),
        command("c4", "review", ""),
    ]);
    assert_eq!(
        user_texts(&events),
        ["/compact keep it short  ", "/cost", "/cost", "/review", "/review"],
        "only /compact's own command line repeats the prompt before it"
    );
}

#[test]
fn a_local_command_written_as_system_lines_shows_as_its_command() {
    // Claude Code 2.1.318 writes `/context` only as `system` `local_command` lines: the command, then its output
    let local = |uuid: &str, content: &str| {
        serde_json::json!({"type": "system", "subtype": "local_command", "uuid": uuid, "content": content}).to_string()
    };
    let named = "<command-name>/context</command-name>\n            <command-message>context</command-message>\n            \
                 <command-args></command-args>";
    let events = parse_owned(&[
        local("c", named),
        local("o", "<local-command-stdout>Context Usage</local-command-stdout>"),
        local("x", "something else"),
        user_line("p", "/cost"),
        local("d", "<command-name>/cost</command-name>"),
    ]);
    assert_eq!(
        user_texts(&events),
        ["/context", "/cost"],
        "the command once, its output and other text nothing, a repeat of the prompt before it nothing"
    );
}

#[test]
fn a_prompt_quoting_command_markup_stays_a_prompt() {
    let quoted = "Explain why <command-name>/compact</command-name> appears here";
    let message_first = "<command-message>compact</command-message>\n<command-name>/compact</command-name>";
    let events = parse_owned(&[user_line("p", quoted), user_line("c", message_first)]);
    assert_eq!(user_texts(&events), [quoted, "/compact"]);
}

#[test]
fn a_resumed_task_keeps_its_tool_on_its_next_notification() {
    let call = r#"{"type":"assistant","uuid":"a","message":{"content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"description":"Count lines"}}]}}"#;
    let notification = |result: &str| {
        let text = format!(
            "<task-notification>\n<tool-use-id>t1</tool-use-id>\n<status>completed</status>\n\
             <result>{result}</result>\n</task-notification>"
        );
        user_line("n", &text)
    };
    let events = parse_owned(&[call.to_owned(), notification("first"), notification("second")]);
    let done = tool(last(&events, "t1"));
    assert_eq!(
        (done.name.as_str(), done.title.as_str(), done.kind),
        ("Agent", "Count lines", ToolKind::Task)
    );
    assert_eq!((done.status, done.output.as_deref()), (ToolStatus::Ok, Some("second")));
}

/// A `Bash` call `t1` and the result Claude Code writes when the command moves to the background.
fn background_bash() -> [String; 2] {
    let call = r#"{"type":"assistant","uuid":"a","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"sleep 60","run_in_background":true}}]}}"#;
    let receipt = serde_json::json!({
        "type": "user", "uuid": "u",
        "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "is_error": false,
            "content": "Command running in background with ID: b1."}]},
        "toolUseResult": {"stdout": "", "stderr": "", "interrupted": false, "backgroundTaskId": "b1"},
    });
    [call.to_owned(), receipt.to_string()]
}

/// The notification Claude Code writes when a background command ends: a summary, no result.
fn bash_notification(status: &str) -> String {
    let text = format!(
        "<task-notification>\n<task-id>b1</task-id>\n<tool-use-id>t1</tool-use-id>\n\
         <output-file>/tmp/b1.output</output-file>\n<status>{status}</status>\n\
         <summary>Background command \"sleep 60\" {status} (exit code 0)</summary>\n</task-notification>"
    );
    user_line("n", &text)
}

#[test]
fn a_background_command_runs_until_its_notification() {
    let [call, receipt] = background_bash();
    let launched = parse_owned(&[call.clone(), receipt.clone()]);
    assert_eq!(tool(last(&launched, "t1")).status, ToolStatus::Running);

    let events = parse_owned(&[call, receipt, bash_notification("completed")]);
    let done = tool(last(&events, "t1"));
    assert_eq!(
        (done.name.as_str(), done.title.as_str(), done.kind),
        ("Bash", "sleep 60", ToolKind::Shell)
    );
    assert_eq!(
        (done.status, done.output.as_deref()),
        (
            ToolStatus::Ok,
            Some("Background command \"sleep 60\" completed (exit code 0)")
        )
    );
}

#[test]
fn a_notification_for_a_finished_call_makes_no_event() {
    let [call, _] = background_bash();
    let result = r#"{"type":"user","uuid":"u","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"done"}]}}"#;
    let events = parse_owned(&[call, result.to_owned(), bash_notification("completed")]);
    let done = tool(last(&events, "t1"));
    assert_eq!((done.name.as_str(), done.output.as_deref()), ("Bash", Some("done")));
}

/// A stop call `s1` of `name`, whose input names task `b1` under `field`.
fn stop_call(name: &str, field: &str) -> String {
    serde_json::json!({
        "type": "assistant", "uuid": "s",
        "message": {"content": [{"type": "tool_use", "id": "s1", "name": name, "input": {field: "b1"}}]},
    })
    .to_string()
}

/// The result Claude Code writes when `TaskStop` stops task `b1`.
fn stop_result() -> String {
    serde_json::json!({
        "type": "user", "uuid": "sr",
        "message": {"content": [{"type": "tool_result", "tool_use_id": "s1",
            "content": r#"{"message":"Successfully stopped task: b1 (sleep 60)","task_id":"b1"}"#}]},
        "toolUseResult": {"message": "Successfully stopped task: b1 (sleep 60)", "task_id": "b1",
            "task_type": "local_bash", "command": "sleep 60"},
    })
    .to_string()
}

/// The result of a stop that failed.
fn failed_stop_result() -> String {
    serde_json::json!({
        "type": "user", "uuid": "sr",
        "message": {"content": [{"type": "tool_result", "tool_use_id": "s1", "is_error": true,
            "content": "<tool_use_error>No task found with ID: b1</tool_use_error>"}]},
        "toolUseResult": "Error: No task found with ID: b1",
    })
    .to_string()
}

const STOPPED: &str = "Successfully stopped task: b1 (sleep 60)";

#[test]
fn a_stopped_background_command_finishes_without_a_notification() {
    let [call, receipt] = background_bash();
    let events = parse_owned(&[call, receipt, stop_call("TaskStop", "task_id"), stop_result()]);
    let stop = last(&events, "s1");
    assert_eq!(tool(stop).status, ToolStatus::Ok);
    let stopped = last(&events, "t1");
    assert_eq!((stopped.cursor, stopped.parent.as_deref()), (stop.cursor, None));
    let stopped = tool(stopped);
    assert_eq!(
        (stopped.name.as_str(), stopped.title.as_str(), stopped.kind),
        ("Bash", "sleep 60", ToolKind::Shell)
    );
    assert_eq!(
        (stopped.status, stopped.output.as_deref()),
        (ToolStatus::Error, Some(STOPPED))
    );
}

#[test]
fn a_stopped_background_agent_finishes_without_a_notification() {
    let call = r#"{"type":"assistant","uuid":"a","message":{"content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"description":"Count lines","run_in_background":true}}]}}"#;
    let receipt = serde_json::json!({
        "type": "user", "uuid": "u",
        "message": {"content": [{"type": "tool_result", "tool_use_id": "t1",
            "content": [{"type": "text", "text": "Async agent launched successfully."}]}]},
        "toolUseResult": {"isAsync": true, "status": "async_launched", "agentId": "b1"},
    });
    let events = parse_owned(&[
        call.to_owned(),
        receipt.to_string(),
        stop_call("TaskStop", "task_id"),
        stop_result(),
    ]);
    let stopped = tool(last(&events, "t1"));
    assert_eq!(
        (stopped.name.as_str(), stopped.title.as_str(), stopped.kind),
        ("Agent", "Count lines", ToolKind::Task)
    );
    assert_eq!(
        (stopped.status, stopped.output.as_deref()),
        (ToolStatus::Error, Some(STOPPED))
    );
}

#[test]
fn a_failed_stop_leaves_its_task_running() {
    let [call, receipt] = background_bash();
    let events = parse_owned(&[call, receipt, stop_call("TaskStop", "task_id"), failed_stop_result()]);
    assert_eq!(tool(last(&events, "s1")).status, ToolStatus::Error);
    assert_eq!(tool(last(&events, "t1")).status, ToolStatus::Running);
}

#[test]
fn a_stop_after_the_task_finished_keeps_its_result() {
    let [call, receipt] = background_bash();
    let events = parse_owned(&[
        call,
        receipt,
        bash_notification("completed"),
        stop_call("TaskStop", "task_id"),
        stop_result(),
    ]);
    let done = tool(last(&events, "t1"));
    assert_eq!(
        (done.status, done.output.as_deref()),
        (
            ToolStatus::Ok,
            Some("Background command \"sleep 60\" completed (exit code 0)")
        )
    );
    assert_eq!(events.iter().filter(|event| event.id == "t1").count(), 2);
}

#[test]
fn a_repeated_stop_changes_nothing() {
    let [call, receipt] = background_bash();
    let again = stop_result().replace("\"s1\"", "\"s2\"");
    let events = parse_owned(&[call, receipt, stop_call("TaskStop", "task_id"), stop_result(), again]);
    assert_eq!(events.iter().filter(|event| event.id == "t1").count(), 2);
    assert_eq!(tool(last(&events, "t1")).output.as_deref(), Some(STOPPED));
}

#[test]
fn a_notification_after_a_stop_keeps_the_tool() {
    let [call, receipt] = background_bash();
    let events = parse_owned(&[
        call,
        receipt,
        stop_call("TaskStop", "task_id"),
        stop_result(),
        bash_notification("killed"),
    ]);
    let done = tool(last(&events, "t1"));
    assert_eq!(
        (done.name.as_str(), done.title.as_str(), done.kind),
        ("Bash", "sleep 60", ToolKind::Shell)
    );
    assert_eq!(
        (done.status, done.output.as_deref()),
        (
            ToolStatus::Error,
            Some("Background command \"sleep 60\" killed (exit code 0)")
        )
    );
}

#[test]
fn a_stop_whose_call_is_not_in_the_window_still_stops_its_task() {
    let [call, receipt] = background_bash();
    let events = parse_owned(&[call, receipt, stop_result()]);
    let stopped = tool(last(&events, "t1"));
    assert_eq!((stopped.name.as_str(), stopped.status), ("Bash", ToolStatus::Error));
    assert_eq!(stopped.output.as_deref(), Some(STOPPED));
}

#[test]
fn a_result_naming_a_task_stops_nothing_unless_its_call_is_a_stop() {
    let [call, receipt] = background_bash();
    let other = r#"{"type":"assistant","uuid":"s","message":{"content":[{"type":"tool_use","id":"s1","name":"TaskOutput","input":{"task_id":"b1"}}]}}"#;
    let events = parse_owned(&[call, receipt, other.to_owned(), stop_result()]);
    assert_eq!(tool(last(&events, "t1")).status, ToolStatus::Running);
}

#[test]
fn kill_shell_and_kill_bash_stop_the_task_their_input_names() {
    for (name, field) in [("KillShell", "shell_id"), ("KillBash", "bash_id")] {
        let [call, receipt] = background_bash();
        let result = serde_json::json!({
            "type": "user", "uuid": "sr",
            "message": {"content": [{"type": "tool_result", "tool_use_id": "s1",
                "content": "Successfully killed shell: b1 (sleep 60)"}]},
            "toolUseResult": {"message": "Successfully killed shell: b1 (sleep 60)"},
        });
        let events = parse_owned(&[call, receipt, stop_call(name, field), result.to_string()]);
        let stopped = tool(last(&events, "t1"));
        assert_eq!(
            (stopped.status, stopped.output.as_deref()),
            (ToolStatus::Error, Some("Successfully killed shell: b1 (sleep 60)")),
            "{name}"
        );
    }
}

#[test]
fn a_notification_with_no_call_in_the_window_still_shows() {
    let events = parse_owned(&[bash_notification("failed")]);
    assert_eq!(events.len(), 1);
    let orphan = tool(last(&events, "t1"));
    assert_eq!((orphan.name.as_str(), orphan.kind), ("", ToolKind::Other));
    assert_eq!(
        (orphan.status, orphan.output.as_deref()),
        (
            ToolStatus::Error,
            Some("Background command \"sleep 60\" failed (exit code 0)")
        )
    );
}

#[test]
fn a_malformed_line_is_unknown_and_the_next_still_parses() {
    let events = parse_lines(&[
        "{\"type\":\"assistant\",\"uuid\":\"a\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"",
        "",
        "[1,2]",
        r#"{"type":"assistant","uuid":"b","message":{"content":[{"type":"text","text":"ok"}]}}"#,
    ]);
    let bodies: Vec<_> = events.iter().map(|event| &event.body).collect();
    assert_eq!(
        bodies,
        [
            &EventBody::Unknown {
                entry: MALFORMED.into()
            },
            &EventBody::Unknown {
                entry: MALFORMED.into()
            },
            &EventBody::Assistant { text: "ok".into() },
        ]
    );
    assert_eq!(events[0].id, "line-0");
}

#[test]
fn an_unknown_type_is_unknown() {
    let events = parse_lines(&[
        r#"{"type":"brand-new","uuid":"x"}"#,
        r#"{"type":"system","subtype":"brand-new","uuid":"y"}"#,
        r#"{"type":"assistant","uuid":"z","message":{"content":[{"type":"brand-new"}]}}"#,
        r#"{"uuid":"w"}"#,
    ]);
    let entries: Vec<_> = events
        .iter()
        .map(|event| match &event.body {
            EventBody::Unknown { entry } => entry.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(entries, ["brand-new", "system:brand-new", "assistant:brand-new", ""]);
}

#[test]
fn a_result_with_no_call_in_the_window_still_shows() {
    let events = parse_lines(&[
        r#"{"type":"user","uuid":"u","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_gone","content":"boom","is_error":true}]}}"#,
    ]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, "toolu_gone");
    let orphan = tool(&events[0]);
    assert_eq!(
        (orphan.name.as_str(), orphan.title.as_str()),
        ("", ""),
        "a page with the call fills these"
    );
    assert_eq!(
        (orphan.status, orphan.output.as_deref()),
        (ToolStatus::Error, Some("boom"))
    );
}

fn call_and_result(output: &str) -> String {
    let call = r#"{"type":"assistant","uuid":"a","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cat big\nsecond line"}}]}}"#;
    let result = serde_json::json!({
        "type": "user", "uuid": "u",
        "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "content": [{"type": "text", "text": output}]}]},
    });
    format!("{call}\n{result}")
}

#[test]
fn oversized_output_is_capped_on_a_char_boundary() {
    // One ASCII byte first puts the cap inside a two-byte `é`.
    let output = format!("a{}", "é".repeat(OUTPUT_CAP));
    let events = parse_bytes(&mut ClaudeParser::new(), call_and_result(&output).as_bytes());
    let done = tool(&events[1]);
    assert_eq!(done.title, "cat big", "a title is one line");
    let capped = done.output.as_deref().unwrap();
    assert_eq!(capped.len(), OUTPUT_CAP - 1);
    assert!(output.starts_with(capped));
    assert_eq!(done.truncated, Some(true));
    let short = parse_bytes(&mut ClaudeParser::new(), call_and_result("fine").as_bytes());
    assert_eq!(tool(&short[1]).truncated, None);
}

#[test]
fn non_utf8_output_keeps_the_rest_of_the_line() {
    let mut bytes = call_and_result("before XX after").into_bytes();
    let at = bytes.windows(2).position(|pair| pair == b"XX").unwrap();
    bytes[at] = 0xff;
    bytes[at + 1] = 0xfe;
    let events = parse_bytes(&mut ClaudeParser::new(), &bytes);
    assert_eq!(
        tool(&events[1]).output.as_deref(),
        Some("before \u{fffd}\u{fffd} after")
    );
}

#[test]
fn an_oversized_line_is_unknown() {
    let mut line = vec![b' '; LINE_CAP + 1];
    line[0] = b'{';
    let events = ClaudeParser::new().parse_line(&line, 7);
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].cursor, &events[0].body),
        (
            7,
            &EventBody::Unknown {
                entry: OVERSIZED.into()
            }
        )
    );
}

#[test]
fn write_and_multi_edit_carry_diffs() {
    let events = parse_lines(&[
        r#"{"type":"assistant","uuid":"a","message":{"content":[{"type":"tool_use","id":"w","name":"Write","input":{"file_path":"/p/new.txt","content":"one\ntwo\n"}}]}}"#,
        r#"{"type":"assistant","uuid":"b","message":{"content":[{"type":"tool_use","id":"m","name":"MultiEdit","input":{"file_path":"/p/a.rs","edits":[{"old_string":"x","new_string":"y"},{"old_string":"p\n","new_string":"q\n"}]}}]}}"#,
    ]);
    let write = tool(&events[0]).diff.as_deref().unwrap();
    assert!(write.contains("+one\n+two\n"), "{write}");
    let multi = tool(&events[1]).diff.as_deref().unwrap();
    assert!(multi.contains("-x\n+y\n") && multi.contains("-p\n+q\n"), "{multi}");
    assert!(!multi.contains("No newline"), "{multi}");
}

#[test]
fn an_api_error_is_a_notice() {
    let events = parse_lines(&[
        r#"{"type":"assistant","uuid":"e","isApiErrorMessage":true,"message":{"model":"<synthetic>","content":[{"type":"text","text":"API Error: 529 overloaded"}]}}"#,
        r#"{"type":"system","subtype":"api_error","uuid":"s","error":{"message":"Connection error."}}"#,
    ]);
    let notices: Vec<_> = events
        .iter()
        .map(|event| match &event.body {
            EventBody::Notice {
                kind: NoticeKind::ApiError,
                text,
                ..
            } => text.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(notices, ["API Error: 529 overloaded", "Connection error."]);
}

#[test]
fn a_prompt_counts_its_images() {
    let events = parse_lines(&[
        r#"{"type":"user","uuid":"u","message":{"content":[{"type":"image","source":{}},{"type":"text","text":"what is this?"}]}}"#,
        r#"{"type":"user","uuid":"v","message":{"content":"plain"}}"#,
    ]);
    assert_eq!(
        events[0].body,
        EventBody::User {
            text: "what is this?".into(),
            images: Some(1)
        }
    );
    assert_eq!(
        events[1].body,
        EventBody::User {
            text: "plain".into(),
            images: None
        }
    );
}

#[test]
fn a_summary_line_is_a_notice() {
    let events = parse_lines(&[r#"{"type":"summary","summary":"Greeter work","leafUuid":"l"}"#]);
    assert_eq!(
        events[0].body,
        EventBody::Notice {
            kind: NoticeKind::Summary,
            text: "Greeter work".into(),
            detail: None
        }
    );
}

#[test]
fn the_ndjson_form_leaves_unset_fields_out() {
    let event = ChatEvent {
        id: "t".into(),
        cursor: 3,
        parent: None,
        body: EventBody::Tool(Tool {
            name: "Read".into(),
            kind: ToolKind::Read,
            title: "/a".into(),
            status: ToolStatus::Running,
            output: None,
            diff: None,
            truncated: None,
        }),
    };
    assert_eq!(
        event.to_ndjson(),
        r#"{"id":"t","cursor":3,"type":"tool","name":"Read","kind":"read","title":"/a","status":"running"}"#
    );
    let live = ChatEvent {
        id: "live".into(),
        cursor: 9,
        parent: None,
        body: EventBody::Live,
    };
    assert_eq!(live.to_ndjson(), r#"{"id":"live","cursor":9,"type":"live"}"#);
}
