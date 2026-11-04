//! The chat's pure parts: the remote command lines, reading the channel's lines, the catalog and the dropdown's
//! ranking.

use ngterm_core::agent_chat::{self as wire, EventBody, Tool, ToolKind, ToolStatus};

use super::*;

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
    let target = format!("[agent][chat][--target][{session}]");
    assert_eq!(
        far_words(&chat_tail(session, Side::Left, None)),
        format!("{target}[--pane][left]")
    );
    let cursor = |at, conversation| Cursor { at, conversation };
    assert_eq!(
        far_words(&chat_tail(session, Side::Right, Some(cursor(42, "c'1")))),
        format!("{target}[--pane][right][--after][42][--conversation][c'1]")
    );
    assert_eq!(
        far_words(&chat_tail(session, Side::Left, Some(cursor(42, "")))),
        format!("{target}[--pane][left][--after][42][--conversation][]"),
        "no conversation known is an empty id, which matches none"
    );
    assert_eq!(
        far_words(&page_tail(session, Side::Left, cursor(120, "c1"), OLDER_PAGE)),
        format!("{target}[--pane][left][--before][120][--conversation][c1][--limit][100]")
    );
    assert_eq!(
        far_words(&commands_tail(session, Side::Right)),
        format!("[agent][commands][--target][{session}][--pane][right]")
    );
    assert!(
        chat_tail(session, Side::Left, None).contains("exec "),
        "the stream's shell does not linger"
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
