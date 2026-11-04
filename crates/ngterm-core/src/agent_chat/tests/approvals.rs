//! The permission prompts `agent chat` interleaves: shown from the record, answered by the record or the transcript.

use ngterm_proto::ControlAgentApproval;

use crate::agent_chat::approvals::Approvals;
use crate::agent_chat::{Approval, ChatEvent, EventBody, NoticeKind, Tool, ToolKind, ToolStatus};

fn denied_txt(since: f64) -> ControlAgentApproval {
    ControlAgentApproval {
        tool: "Bash".to_owned(),
        title: "Create empty file denied.txt".to_owned(),
        detail: Some("touch denied.txt".to_owned()),
        tool_use_id: None,
        since,
    }
}

fn shown(approval: &ControlAgentApproval) -> Approval {
    Approval {
        tool: approval.tool.clone(),
        title: approval.title.clone(),
        detail: approval.detail.clone(),
        tool_use_id: approval.tool_use_id.clone(),
    }
}

fn event(id: &str, cursor: u64, body: EventBody) -> ChatEvent {
    ChatEvent {
        id: id.to_owned(),
        cursor,
        parent: None,
        body,
    }
}

fn bash(id: &str, cursor: u64, title: &str, status: ToolStatus) -> ChatEvent {
    event(
        id,
        cursor,
        EventBody::Tool(Tool {
            name: "Bash".to_owned(),
            kind: ToolKind::Shell,
            title: title.to_owned(),
            status,
            output: None,
            diff: None,
            truncated: None,
        }),
    )
}

/// Reads `events` in order: the last `approval_done` they make.
fn read(approvals: &mut Approvals, events: &[ChatEvent]) -> Option<ChatEvent> {
    let mut done = None;
    for event in events {
        if let Some(event) = approvals.read(event) {
            done = Some(event);
        }
    }
    done
}

fn live(cursor: u64) -> ChatEvent {
    event("live", cursor, EventBody::Live)
}

fn types(events: &[ChatEvent]) -> Vec<(&str, &str, u64)> {
    events
        .iter()
        .map(|event| {
            let kind = match event.body {
                EventBody::Approval(_) => "approval",
                EventBody::ApprovalDone(_) => "approval_done",
                _ => "other",
            };
            (kind, event.id.as_str(), event.cursor)
        })
        .collect()
}

#[test]
fn a_live_approval_shows_once_at_the_stream_position_and_the_record_dropping_it_ends_it() {
    let mut approvals = Approvals::new();
    assert_eq!(
        read(&mut approvals, &[bash("t1", 10, "ls", ToolStatus::Ok), live(500)]),
        None
    );
    let approval = denied_txt(1_762_207_306.5);
    let shown_events = approvals.observe(Some(&approval));
    assert_eq!(types(&shown_events), [("approval", "approval-1762207306.5", 500)]);
    assert_eq!(shown_events[0].body, EventBody::Approval(shown(&approval)));
    assert_eq!(shown_events[0].parent, None);
    // the same candidate again is not news
    assert!(approvals.observe(Some(&approval)).is_empty());
    let done = approvals.observe(None);
    assert_eq!(types(&done), [("approval_done", "approval-1762207306.5", 500)]);
    assert_eq!(done[0].body, EventBody::ApprovalDone(shown(&approval)));
    assert!(approvals.observe(None).is_empty());
}

#[test]
fn the_transcript_writing_the_calls_result_answers_it_and_the_same_candidate_never_shows_again() {
    let mut approvals = Approvals::new();
    read(
        &mut approvals,
        &[bash("toolu_1", 700, "touch denied.txt", ToolStatus::Running), live(900)],
    );
    let approval = denied_txt(5.0);
    assert_eq!(approvals.observe(Some(&approval)).len(), 1);
    // another call finishing, or this one still running, answers nothing
    assert_eq!(
        read(&mut approvals, &[bash("toolu_0", 950, "ls", ToolStatus::Ok)]),
        None
    );
    assert_eq!(
        read(
            &mut approvals,
            &[bash("toolu_1", 960, "touch denied.txt", ToolStatus::Running)]
        ),
        None
    );
    let done = read(
        &mut approvals,
        &[bash("toolu_1", 1000, "touch denied.txt", ToolStatus::Error)],
    );
    assert_eq!(
        done.as_ref().map(|done| types(std::slice::from_ref(done))),
        Some(vec![("approval_done", "approval-5", 1000)])
    );
    // the status has not left `blocked` yet: the record still holds the answered candidate
    assert!(approvals.observe(Some(&approval)).is_empty());
    assert!(approvals.observe(None).is_empty());
    // a new candidate is news
    let next = approvals.observe(Some(&denied_txt(6.0)));
    assert_eq!(types(&next), [("approval", "approval-6", 1000)]);
}

#[test]
fn an_interrupt_notice_answers_it() {
    let mut approvals = Approvals::new();
    read(&mut approvals, &[live(100)]);
    approvals.observe(Some(&denied_txt(1.0)));
    let interrupt = event(
        "n1",
        200,
        EventBody::Notice {
            kind: NoticeKind::Interrupt,
            text: "Request interrupted by user for tool use".to_owned(),
            detail: None,
        },
    );
    let done = read(&mut approvals, &[interrupt]);
    assert!(matches!(done.map(|done| done.body), Some(EventBody::ApprovalDone(_))));
}

#[test]
fn a_known_tool_use_id_matches_by_id_alone() {
    let mut approvals = Approvals::new();
    read(&mut approvals, &[live(100)]);
    let approval = ControlAgentApproval {
        tool_use_id: Some("toolu_9".to_owned()),
        ..denied_txt(1.0)
    };
    approvals.observe(Some(&approval));
    assert_eq!(
        read(
            &mut approvals,
            &[bash("toolu_8", 150, "touch denied.txt", ToolStatus::Error)]
        ),
        None
    );
    assert!(read(&mut approvals, &[bash("toolu_9", 160, "other", ToolStatus::Ok)]).is_some());
}

#[test]
fn a_codex_call_matches_by_its_kind_and_command_whatever_the_tool_is_named() {
    let mut approvals = Approvals::new();
    read(&mut approvals, &[live(100)]);
    approvals.observe(Some(&ControlAgentApproval {
        tool: "Bash".to_owned(),
        title: "Check the network".to_owned(),
        detail: Some("curl -sI https://example.com\n".to_owned()),
        tool_use_id: None,
        since: 1.0,
    }));
    let mut exec = bash("exec-1", 300, "curl -sI https://example.com", ToolStatus::Ok);
    if let EventBody::Tool(tool) = &mut exec.body {
        tool.name = "exec_command".to_owned();
    }
    assert!(read(&mut approvals, &[exec]).is_some());
}

#[test]
fn a_new_candidate_ends_the_shown_one_first() {
    let mut approvals = Approvals::new();
    read(&mut approvals, &[live(100)]);
    approvals.observe(Some(&denied_txt(1.0)));
    let events = approvals.observe(Some(&denied_txt(2.0)));
    assert_eq!(
        types(&events),
        [("approval_done", "approval-1", 100), ("approval", "approval-2", 100)]
    );
}

#[test]
fn a_reset_forgets_what_was_shown_so_a_live_one_shows_again() {
    let mut approvals = Approvals::new();
    read(&mut approvals, &[live(100)]);
    let approval = denied_txt(1.0);
    approvals.observe(Some(&approval));
    assert_eq!(
        read(&mut approvals, &[event("reset", 0, EventBody::Reset), live(40)]),
        None
    );
    assert_eq!(
        types(&approvals.observe(Some(&approval))),
        [("approval", "approval-1", 40)]
    );
}

#[test]
fn a_subagents_events_do_not_move_the_position() {
    let mut approvals = Approvals::new();
    let mut sub = bash("a:line-9000", 9000, "ls", ToolStatus::Ok);
    sub.parent = Some("toolu_task".to_owned());
    read(&mut approvals, &[live(100), sub]);
    assert_eq!(
        types(&approvals.observe(Some(&denied_txt(1.0)))),
        [("approval", "approval-1", 100)]
    );
}
