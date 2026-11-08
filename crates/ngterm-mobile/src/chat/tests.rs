//! The chat's pure parts: the remote command lines, reading the channel's lines, the catalog and the dropdown's
//! ranking; the screen matchers on the recorded agent screens ([`matchers`]) and the actions' plans and keys
//! ([`input`]).

use ngterm_core::agent_chat::account::Usage;
use ngterm_core::agent_chat::{self as wire, EventBody, Tool, ToolKind, ToolStatus};

use super::*;

mod input;
pub(crate) mod matchers;
mod upload;

fn command(name: &str, description: &str) -> AgentCommand {
    AgentCommand {
        name: name.to_owned(),
        description: description.to_owned(),
        kind: AgentCommandKind::Builtin,
        argument_hint: None,
    }
}

fn names(commands: &[AgentCommand]) -> Vec<&str> {
    commands.iter().map(|command| command.name.as_str()).collect()
}

/// What the far shell runs of `tail`: the words a stand-in `ngtermctl` on the widened `PATH` was called with.
#[cfg(unix)]
fn far_words(tail: &str) -> String {
    use std::os::unix::fs::PermissionsExt as _;

    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let cli = bin.join("ngtermctl");
    std::fs::write(
        &cli,
        "#!/bin/sh\nfor word in \"$@\"; do printf '[%s]' \"$word\"; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    // the tail is what the account's shell is handed: run it as that shell would
    let ran = std::process::Command::new("/bin/sh")
        .args(["-c", tail])
        .env_clear()
        .env("HOME", home.path())
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(0), "{ran:?}");
    String::from_utf8_lossy(&ran.stdout).into_owned()
}

#[cfg(unix)]
#[test]
fn the_remote_lines_run_the_far_ngtermctl_agent_for_the_session_and_side() {
    let session = "00000000-0000-0000-0000-0000000000A1";
    let target = format!("[agent][chat][--target={session}]");
    assert_eq!(
        far_words(&chat_tail(session, Side::Left, None)),
        format!("{target}[--pane][left]")
    );
    let cursor = |at, conversation| Cursor { at, conversation };
    assert_eq!(
        far_words(&chat_tail(session, Side::Right, Some(cursor(42, "c'1")))),
        format!("{target}[--pane][right][--after][42][--conversation=c'1]")
    );
    assert_eq!(
        far_words(&chat_tail(session, Side::Left, Some(cursor(42, "")))),
        format!("{target}[--pane][left][--after][42][--conversation=]"),
        "no conversation known is an empty id, which matches none"
    );
    assert_eq!(
        far_words(&page_tail(session, Side::Left, cursor(120, "c1"), OLDER_PAGE)),
        format!("{target}[--pane][left][--before][120][--conversation=c1][--limit][100]")
    );
    assert_eq!(
        far_words(&commands_tail(session, Side::Right)),
        format!("[agent][commands][--target={session}][--pane][right]")
    );
    assert!(
        chat_tail(session, Side::Left, None).contains("exec "),
        "the stream's shell does not linger"
    );
    // a value that starts with `-` stays one word with its option, so the far parser can't take it for an option
    assert_eq!(
        far_words(&page_tail("-s", Side::Left, cursor(1, "-c"), OLDER_PAGE)),
        "[agent][chat][--target=-s][--pane][left][--before][1][--conversation=-c][--limit][100]"
    );
}

#[test]
fn lines_are_read_across_chunks_and_one_that_is_not_an_event_is_skipped() {
    let mut lines = Lines::default();
    assert!(lines.take(br#"{"id":"live","cursor":7,"#).is_empty());
    let events = lines.take(b"\"type\":\"live\"}\nnot json\n{\"id\":\"x\",\"cursor\":1,\"type\":\"from-later\"}\n");
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].cursor, &events[0].body), (7, &EventBody::Live));
}

#[test]
fn a_line_past_the_cap_is_skipped_to_its_newline_and_reading_goes_on() {
    let mut lines = Lines::default();
    let huge = vec![b'x'; EVENT_LINE_CAP + 1];
    assert!(lines.take(&huge).is_empty());
    assert!(lines.partial.is_empty(), "never held whole");
    assert!(lines.take(b"xxxx").is_empty());
    let events = lines.take(b"tail\n{\"id\":\"live\",\"cursor\":3,\"type\":\"live\"}\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cursor, 3);
}

#[test]
fn a_message_as_long_as_the_origin_reads_is_read_across_many_reads() {
    // a reply of several MiB, under the transcript line cap: the origin sends it whole, and so it is read
    let text = "y".repeat(wire::event::LINE_CAP - 4096);
    let line = format!("{{\"id\":\"a1\",\"cursor\":9,\"type\":\"assistant\",\"text\":\"{text}\"}}\n");
    assert!(line.len() > 1024 * 1024 && line.len() <= EVENT_LINE_CAP);
    let mut lines = Lines::default();
    let mut events = Vec::new();
    for chunk in line.as_bytes().chunks(32 * 1024) {
        events.extend(lines.take(chunk));
    }
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cursor, 9);
    assert!(matches!(&events[0].body, EventBody::Assistant { text: shown } if *shown == text));
}

#[test]
fn the_catalog_keeps_each_entry_it_reads_and_refuses_what_is_not_a_list() {
    let catalog = r#"[{"name":"compact","description":"Clear","kind":"builtin","argument_hint":"[x]"},
                      {"name":"$deploy","description":"","kind":"skill"},
                      {"name":"new","description":"","kind":"from-later"},
                      {"description":"no name","kind":"skill"}]"#;
    let commands = parse_catalog(catalog).unwrap();
    assert_eq!(names(&commands), ["compact", "$deploy"]);
    assert_eq!(commands[0].argument_hint.as_deref(), Some("[x]"));
    assert_eq!(commands[1].kind, AgentCommandKind::Skill);
    assert!(parse_catalog("Error: no agent in this pane").is_err());
    assert_eq!(parse_catalog("[]"), Ok(Vec::new()));
}

#[test]
fn the_dropdown_ranks_by_name_as_the_picker_does() {
    let items = vec![
        command("review", "Review a pull request"),
        command("compact", "Clear conversation history but keep a summary"),
        command("$code-review", "A skill"),
        command("context", "Visualize current context usage"),
    ];
    assert_eq!(
        names(&filter_commands("  ".to_owned(), items.clone())),
        ["$code-review", "compact", "context", "review"],
        "a blank query lists them all A→Z"
    );
    assert_eq!(
        names(&filter_commands("co".to_owned(), items.clone())),
        ["compact", "context", "$code-review"]
    );
    assert_eq!(
        names(&filter_commands("rev".to_owned(), items.clone())),
        ["review", "$code-review"]
    );
    assert!(
        filter_commands("summary".to_owned(), items.clone()).is_empty(),
        "a description's words do not match"
    );
    assert!(filter_commands("zzz".to_owned(), items).is_empty());
}

#[test]
fn a_wire_event_becomes_its_record() {
    let event = wire::ChatEvent {
        id: "toolu_1".to_owned(),
        cursor: 9,
        parent: Some("toolu_0".to_owned()),
        body: EventBody::Tool(Tool {
            name: "Edit".to_owned(),
            kind: ToolKind::Edit,
            title: "src/lib.rs".to_owned(),
            status: ToolStatus::Ok,
            output: None,
            diff: Some("@@ -1 +1 @@".to_owned()),
            truncated: None,
        }),
    };
    assert_eq!(
        ChatEvent::from(event),
        ChatEvent {
            id: "toolu_1".to_owned(),
            cursor: 9,
            parent: Some("toolu_0".to_owned()),
            body: ChatBody::Tool {
                tool: ChatTool {
                    name: "Edit".to_owned(),
                    kind: ChatToolKind::Edit,
                    title: "src/lib.rs".to_owned(),
                    status: ChatToolStatus::Ok,
                    output: None,
                    diff: Some("@@ -1 +1 @@".to_owned()),
                    truncated: false,
                },
            },
        }
    );
    let account = wire::ChatEvent {
        id: "account".to_owned(),
        cursor: 4,
        parent: None,
        body: EventBody::Account {
            email: Some("dev@example.com".to_owned()),
            weekly: Some(Usage {
                used_percent: 72,
                resets_at: Some(9),
            }),
        },
    };
    assert_eq!(
        ChatEvent::from(account).body,
        ChatBody::Account {
            email: Some("dev@example.com".to_owned()),
            weekly: Some(ChatUsage {
                used_percent: 72,
                resets_at: Some(9)
            }),
        }
    );
    let user = wire::ChatEvent {
        id: "line-0".to_owned(),
        cursor: 0,
        parent: None,
        body: EventBody::User {
            text: "hi".to_owned(),
            images: None,
        },
    };
    assert_eq!(
        ChatEvent::from(user).body,
        ChatBody::User {
            text: "hi".to_owned(),
            images: 0
        }
    );
}

/// The records `lines` (the origin's NDJSON) become, as the app reads them.
fn records(lines: &[serde_json::Value]) -> Vec<ChatBody> {
    let output = lines.iter().fold(String::new(), |mut output, line| {
        output.push_str(&line.to_string());
        output.push('\n');
        output
    });
    parse_events(&output)
        .into_iter()
        .map(|event| ChatEvent::from(event).body)
        .collect()
}

#[test]
fn each_body_the_origin_writes_reaches_the_app_with_its_fields() {
    let bodies = records(&[
        serde_json::json!({"id": "c", "cursor": 0, "type": "conversation", "agent": "codex",
                           "conversation_id": "r1", "cwd": "/repo", "model": "gpt-5"}),
        serde_json::json!({"id": "u", "cursor": 1, "type": "user", "text": "look", "images": 2}),
        serde_json::json!({"id": "a", "cursor": 2, "type": "assistant", "text": "**done**"}),
        serde_json::json!({"id": "t", "cursor": 3, "type": "thinking", "text": "Planning"}),
        serde_json::json!({"id": "x", "cursor": 4, "type": "unknown", "entry": "malformed"}),
        serde_json::json!({"id": "account", "cursor": 5, "type": "account"}),
        serde_json::json!({"id": "reset", "cursor": 6, "type": "reset"}),
        serde_json::json!({"id": "live", "cursor": 7, "type": "live"}),
    ]);
    assert_eq!(
        bodies,
        [
            ChatBody::Conversation {
                agent: "codex".to_owned(),
                conversation_id: "r1".to_owned(),
                cwd: Some("/repo".to_owned()),
                model: Some("gpt-5".to_owned()),
            },
            ChatBody::User {
                text: "look".to_owned(),
                images: 2,
            },
            ChatBody::Assistant {
                text: "**done**".to_owned()
            },
            ChatBody::Thinking {
                text: "Planning".to_owned()
            },
            ChatBody::Unknown {
                entry: "malformed".to_owned()
            },
            ChatBody::Account {
                email: None,
                weekly: None
            },
            ChatBody::Reset,
            ChatBody::Live,
        ]
    );
}

#[test]
fn a_question_and_its_options_reach_the_app_in_order() {
    let bodies = records(&[serde_json::json!({
        "id": "toolu_q", "cursor": 3, "type": "question", "status": "error", "output": "refused",
        "questions": [
            {"question": "Which database?", "header": "DB", "multi_select": false,
             "options": [{"label": "Postgres", "description": "the default"}, {"label": "SQLite"}]},
            {"question": "Extras?", "multi_select": true, "options": []},
        ],
    })]);
    assert_eq!(
        bodies,
        [ChatBody::Question {
            question: ChatQuestion {
                questions: vec![
                    ChatQuestionItem {
                        question: "Which database?".to_owned(),
                        header: Some("DB".to_owned()),
                        multi_select: false,
                        options: vec![
                            ChatQuestionOption {
                                label: "Postgres".to_owned(),
                                description: Some("the default".to_owned()),
                            },
                            ChatQuestionOption {
                                label: "SQLite".to_owned(),
                                description: None,
                            },
                        ],
                    },
                    ChatQuestionItem {
                        question: "Extras?".to_owned(),
                        header: None,
                        multi_select: true,
                        options: Vec::new(),
                    },
                ],
                status: ChatToolStatus::Error,
                output: Some("refused".to_owned()),
            },
        }]
    );
}

#[test]
fn an_approval_and_its_answer_carry_the_call_they_ask_about() {
    let approval = serde_json::json!({"tool": "Bash", "title": "rm -rf build", "detail": "Remove the build",
                                      "tool_use_id": "toolu_7", "cwd": "/repo"});
    let mut asked = approval.clone();
    asked["id"] = "approval".into();
    asked["cursor"] = 8.into();
    asked["type"] = "approval".into();
    let mut done = asked.clone();
    done["type"] = "approval_done".into();
    let bare = serde_json::json!({"id": "approval", "cursor": 9, "type": "approval", "tool": "WebFetch",
                                  "title": "https://example.com"});
    let expected = ChatApproval {
        tool: "Bash".to_owned(),
        title: "rm -rf build".to_owned(),
        detail: Some("Remove the build".to_owned()),
        tool_use_id: Some("toolu_7".to_owned()),
        cwd: Some("/repo".to_owned()),
    };
    assert_eq!(
        records(&[asked, done, bare]),
        [
            ChatBody::Approval {
                approval: expected.clone()
            },
            ChatBody::ApprovalDone { approval: expected },
            ChatBody::Approval {
                approval: ChatApproval {
                    tool: "WebFetch".to_owned(),
                    title: "https://example.com".to_owned(),
                    detail: None,
                    tool_use_id: None,
                    cwd: None,
                },
            },
        ]
    );
}

#[test]
fn every_tool_kind_status_and_notice_kind_keeps_its_name() {
    let tool = |kind: &str, status: &str| {
        serde_json::json!({"id": "t", "cursor": 1, "type": "tool", "name": "X", "kind": kind, "title": "x",
                           "status": status, "output": "out", "truncated": true})
    };
    let kinds = records(&[
        tool("shell", "running"),
        tool("read", "ok"),
        tool("search", "error"),
        tool("web", "ok"),
        tool("task", "ok"),
        tool("other", "ok"),
    ]);
    let seen: Vec<(ChatToolKind, ChatToolStatus, bool, Option<String>)> = kinds
        .into_iter()
        .map(|body| match body {
            ChatBody::Tool { tool } => (tool.kind, tool.status, tool.truncated, tool.output),
            other => panic!("not a tool: {other:?}"),
        })
        .collect();
    let out = Some("out".to_owned());
    assert_eq!(
        seen,
        [
            (ChatToolKind::Shell, ChatToolStatus::Running, true, out.clone()),
            (ChatToolKind::Read, ChatToolStatus::Ok, true, out.clone()),
            (ChatToolKind::Search, ChatToolStatus::Error, true, out.clone()),
            (ChatToolKind::Web, ChatToolStatus::Ok, true, out.clone()),
            (ChatToolKind::Task, ChatToolStatus::Ok, true, out.clone()),
            (ChatToolKind::Other, ChatToolStatus::Ok, true, out),
        ]
    );

    let notice = |kind: &str| serde_json::json!({"id": "n", "cursor": 2, "type": "notice", "kind": kind, "text": "t", "detail": "d"});
    let notices: Vec<ChatNoticeKind> = records(&[
        notice("compaction"),
        notice("interrupt"),
        notice("api_error"),
        notice("summary"),
    ])
    .into_iter()
    .map(|body| match body {
        ChatBody::Notice { kind, text, detail } => {
            assert_eq!((text.as_str(), detail.as_deref()), ("t", Some("d")));
            kind
        }
        other => panic!("not a notice: {other:?}"),
    })
    .collect();
    assert_eq!(
        notices,
        [
            ChatNoticeKind::Compaction,
            ChatNoticeKind::Interrupt,
            ChatNoticeKind::ApiError,
            ChatNoticeKind::Summary,
        ]
    );
}

#[test]
fn the_catalog_names_where_each_entry_comes_from() {
    let catalog = r#"[{"name":"compact","description":"","kind":"builtin"},
                      {"name":"deploy","description":"","kind":"skill"},
                      {"name":"ship","description":"Ship it","kind":"command","argument_hint":"[branch]"},
                      {"name":"lint:fix","description":"","kind":"plugin"}]"#;
    let kinds: Vec<(String, AgentCommandKind)> = parse_catalog(catalog)
        .unwrap()
        .into_iter()
        .map(|command| (command.name, command.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("compact".to_owned(), AgentCommandKind::Builtin),
            ("deploy".to_owned(), AgentCommandKind::Skill),
            ("ship".to_owned(), AgentCommandKind::Command),
            ("lint:fix".to_owned(), AgentCommandKind::Plugin),
        ]
    );
}
