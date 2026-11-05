//! `agent.report` and `agent.get`: the per-pane agent record on the fakes (`commands::agent`).

use ngterm_host::ProcessStat;
use ngterm_host::fakes::{FakeProcessTable, FakeSurface, FakeSurfaceProbe};
use ngterm_proto::command::{AgentCmd, SessionCmd};
use ngterm_proto::{AgentApproval, AgentReport, Args, Command, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Fixture, UNKNOWN, args, failure, send, short_tempdir, wire};
use crate::persist::{AppSettings, LibraryPersistence};
use crate::{Core, CoreConfig, CoreState};

const CLAUDE_A: &str = "/home/user/.claude/projects/-home-user-project/aaaa.jsonl";
const CLAUDE_B: &str = "/home/user/.claude/projects/-home-user-project/bbbb.jsonl";
const CODEX_A: &str = "/home/user/.codex/sessions/2026/11/03/rollout-2026-11-03T23-01-46-cccc.jsonl";

fn conversation(agent: &str, event: &str, transcript: &str, id: &str) -> AgentReport {
    AgentReport {
        agent: agent.to_owned(),
        event: event.to_owned(),
        transcript_path: Some(transcript.to_owned()),
        conversation_id: Some(id.to_owned()),
        approval: None,
    }
}

fn start(transcript: &str, id: &str) -> AgentReport {
    conversation("claude", "SessionStart", transcript, id)
}

fn permission(tool: &str, command: &str) -> AgentReport {
    AgentReport {
        approval: Some(AgentApproval {
            tool: tool.to_owned(),
            title: format!("Run {command}"),
            detail: Some(command.to_owned()),
            tool_use_id: None,
            cwd: None,
        }),
        ..conversation("claude", "PermissionRequest", CLAUDE_A, "aaaa")
    }
}

fn end(id: &str) -> AgentReport {
    AgentReport {
        conversation_id: Some(id.to_owned()),
        ..conversation("claude", "SessionEnd", CLAUDE_A, id)
    }
}

/// Addresses the pane as the hooks do: its token and its role.
fn pane_args(token: &str, role: &str) -> Args {
    args(|a| {
        a.pane_id = Some(token.to_owned());
        a.pane = Some(role.to_owned());
    })
}

fn send_report(fixture: &mut Fixture, session: Uuid, pane: Args, report: AgentReport) -> Response {
    let mut args = pane;
    args.report = Some(Box::new(report));
    fixture.send(Command::Agent(AgentCmd::Report), Some(&wire(session)), args)
}

fn get(fixture: &mut Fixture, session: Uuid, pane: Args) -> Response {
    fixture.send(Command::Agent(AgentCmd::Get), Some(&wire(session)), pane)
}

/// The `agent` of a successful `agent.get`.
fn agent_of(response: &Response) -> Value {
    assert!(response.ok, "{response:?}");
    response.result.as_ref().expect("a result")["agent"].clone()
}

fn reported(session: Uuid) -> Response {
    Response::with_result(json!({"id": wire(session)}))
}

fn status(fixture: &mut Fixture, session: Uuid, status: &str, pane: &str) {
    let response = fixture.session(
        SessionCmd::Status,
        Some(&wire(session)),
        args(|a| {
            a.status = Some(status.to_owned());
            a.pane = Some(pane.to_owned());
        }),
    );
    assert!(response.ok, "{response:?}");
}

/// The fixture's one session with its main pane's token.
fn one_pane() -> (Fixture, Uuid, String) {
    let fixture = Fixture::new(0);
    let session = fixture.sessions[0];
    let token = wire(fixture.store().session(session).expect("the session").pane_identity);
    (fixture, session, token)
}

/// Splits `session`, answering the split pane's token.
fn split(fixture: &mut Fixture, session: Uuid) -> String {
    let response = fixture.session(
        SessionCmd::Split,
        Some(&wire(session)),
        args(|a| a.mode = Some("on".into())),
    );
    assert!(response.ok, "{response:?}");
    let identity = fixture.store().session(session).and_then(|s| s.split_pane_identity);
    wire(identity.expect("a split pane"))
}

#[test]
fn a_report_sets_the_record_and_a_reassert_keeps_it() {
    let (mut fixture, session, token) = one_pane();
    let left = || pane_args(&token, "left");
    assert_eq!(get(&mut fixture, session, left()), failure("no agent in this pane"));
    assert_eq!(
        send_report(&mut fixture, session, left(), start(CLAUDE_A, "aaaa")),
        reported(session)
    );
    let first = agent_of(&get(&mut fixture, session, left()));
    assert_eq!(
        [
            &first["agent"],
            &first["pane"],
            &first["transcriptPath"],
            &first["conversationId"]
        ],
        [&json!("claude"), &json!("left"), &json!(CLAUDE_A), &json!("aaaa")]
    );
    assert!(first["updatedAt"].as_f64().is_some_and(|at| at > 0.0), "{first}");
    assert_eq!(first.get("approval"), None);
    // re-asserted by the next prompt: nothing changes, not even the time
    let prompt = conversation("claude", "UserPromptSubmit", CLAUDE_A, "aaaa");
    assert!(send_report(&mut fixture, session, left(), prompt).ok);
    assert_eq!(agent_of(&get(&mut fixture, session, left())), first);
}

#[test]
fn a_new_path_replaces_the_record_and_only_its_own_end_clears_it() {
    let (mut fixture, session, token) = one_pane();
    let left = || pane_args(&token, "left");
    assert!(send_report(&mut fixture, session, left(), start(CLAUDE_A, "aaaa")).ok);
    // /clear: the old conversation ends, a new one starts in a new file
    assert!(send_report(&mut fixture, session, left(), end("aaaa")).ok);
    assert!(send_report(&mut fixture, session, left(), start(CLAUDE_B, "bbbb")).ok);
    let second = agent_of(&get(&mut fixture, session, left()));
    assert_eq!(
        [&second["transcriptPath"], &second["conversationId"]],
        [&json!(CLAUDE_B), &json!("bbbb")]
    );
    // another conversation's end (Codex ends every thread it loaded) leaves it
    assert!(send_report(&mut fixture, session, left(), end("aaaa")).ok);
    assert_eq!(agent_of(&get(&mut fixture, session, left())), second);
    assert!(send_report(&mut fixture, session, left(), end("bbbb")).ok);
    assert_eq!(get(&mut fixture, session, left()), failure("no agent in this pane"));
}

#[test]
fn a_codex_record_reads_back_as_codex() {
    let (mut fixture, session, token) = one_pane();
    let report = conversation("codex", "SessionStart", CODEX_A, "cccc");
    assert!(send_report(&mut fixture, session, pane_args(&token, "left"), report).ok);
    let agent = agent_of(&get(&mut fixture, session, pane_args(&token, "left")));
    assert_eq!(
        (agent["agent"].clone(), agent["transcriptPath"].clone()),
        (json!("codex"), json!(CODEX_A))
    );
}

#[test]
fn a_candidate_is_an_approval_only_while_the_pane_is_blocked_and_done_once_it_leaves() {
    let (mut fixture, session, token) = one_pane();
    let left = || pane_args(&token, "left");
    assert!(send_report(&mut fixture, session, left(), start(CLAUDE_A, "aaaa")).ok);
    assert!(send_report(&mut fixture, session, left(), permission("Bash", "touch allowed.txt")).ok);
    // the hook's report can come before the adapter's `blocked`
    assert_eq!(agent_of(&get(&mut fixture, session, left())).get("approval"), None);
    status(&mut fixture, session, "blocked", "left");
    let approval = agent_of(&get(&mut fixture, session, left()))["approval"].clone();
    assert_eq!(approval["tool"], "Bash");
    assert_eq!(approval["title"], "Run touch allowed.txt");
    assert_eq!(approval["detail"], "touch allowed.txt");
    assert!(approval["since"].as_f64().is_some(), "{approval}");
    assert_eq!(approval.get("toolUseId"), None);
    // answered: the status leaves blocked, and a later block is another prompt's
    status(&mut fixture, session, "active", "left");
    assert_eq!(agent_of(&get(&mut fixture, session, left())).get("approval"), None);
    status(&mut fixture, session, "blocked", "left");
    assert_eq!(agent_of(&get(&mut fixture, session, left())).get("approval"), None);
    // a prompt reported while the pane is already blocked is live at once
    assert!(send_report(&mut fixture, session, left(), permission("Bash", "touch second.txt")).ok);
    let approval = agent_of(&get(&mut fixture, session, left()))["approval"].clone();
    assert_eq!(approval["detail"], "touch second.txt");
}

#[test]
fn a_commit_ends_a_candidate_whose_status_left_blocked_between_two_reads() {
    let (mut fixture, session, token) = one_pane();
    let left = || pane_args(&token, "left");
    assert!(send_report(&mut fixture, session, left(), start(CLAUDE_A, "aaaa")).ok);
    assert!(send_report(&mut fixture, session, left(), permission("Bash", "ls")).ok);
    for state in ["blocked", "active", "blocked"] {
        status(&mut fixture, session, state, "left");
        let _ = fixture.core.commit();
    }
    assert_eq!(agent_of(&get(&mut fixture, session, left())).get("approval"), None);
}

#[test]
fn a_new_prompt_or_conversation_drops_the_candidate() {
    let (mut fixture, session, token) = one_pane();
    let left = || pane_args(&token, "left");
    assert!(send_report(&mut fixture, session, left(), start(CLAUDE_A, "aaaa")).ok);
    // Auto Review approved it: the status never went blocked
    assert!(send_report(&mut fixture, session, left(), permission("Bash", "curl")).ok);
    let prompt = conversation("claude", "UserPromptSubmit", CLAUDE_A, "aaaa");
    assert!(send_report(&mut fixture, session, left(), prompt).ok);
    status(&mut fixture, session, "blocked", "left");
    assert_eq!(agent_of(&get(&mut fixture, session, left())).get("approval"), None);
}

#[test]
fn a_candidate_its_pane_moved_past_never_goes_live_on_a_later_block() {
    let (mut fixture, session, token) = one_pane();
    let left = || pane_args(&token, "left");
    let codex = |event: &str| conversation("codex", event, CODEX_A, "cccc");
    assert!(send_report(&mut fixture, session, left(), codex("SessionStart")).ok);
    // Auto Review approved it: no dialog, the command's PostToolUse writes active, then the turn ends with a question
    let approved = AgentReport {
        approval: permission("Bash", "curl example.com").approval,
        ..codex("PermissionRequest")
    };
    assert!(send_report(&mut fixture, session, left(), approved).ok);
    status(&mut fixture, session, "active", "left");
    status(&mut fixture, session, "blocked", "left");
    assert_eq!(agent_of(&get(&mut fixture, session, left())).get("approval"), None);
}

#[test]
fn a_write_another_panes_block_refuses_still_ends_the_writers_candidate() {
    let (mut fixture, session, left_token) = one_pane();
    let _ = split(&mut fixture, session);
    let left = || pane_args(&left_token, "left");
    assert!(send_report(&mut fixture, session, left(), start(CLAUDE_A, "aaaa")).ok);
    assert!(send_report(&mut fixture, session, left(), permission("Bash", "ls")).ok);
    status(&mut fixture, session, "blocked", "right");
    let refused = fixture.session(
        SessionCmd::Status,
        Some(&wire(session)),
        args(|a| {
            a.status = Some("active".to_owned());
            a.pane = Some("left".to_owned());
        }),
    );
    assert!(!refused.ok, "{refused:?}");
    status(&mut fixture, session, "idle", "right");
    status(&mut fixture, session, "blocked", "left");
    assert_eq!(agent_of(&get(&mut fixture, session, left())).get("approval"), None);
    // the other pane's writes leave a candidate alone
    status(&mut fixture, session, "idle", "left");
    assert!(send_report(&mut fixture, session, left(), permission("Bash", "date")).ok);
    status(&mut fixture, session, "active", "right");
    status(&mut fixture, session, "blocked", "left");
    assert_eq!(
        agent_of(&get(&mut fixture, session, left()))["approval"]["detail"],
        "date"
    );
}

#[test]
fn another_panes_block_is_not_this_panes_approval() {
    let (mut fixture, session, left_token) = one_pane();
    let right_token = split(&mut fixture, session);
    let right = || pane_args(&right_token, "right");
    assert!(send_report(&mut fixture, session, right(), start(CLAUDE_A, "aaaa")).ok);
    assert!(send_report(&mut fixture, session, right(), permission("Bash", "ls")).ok);
    status(&mut fixture, session, "blocked", "left");
    assert_eq!(agent_of(&get(&mut fixture, session, right())).get("approval"), None);
    assert_eq!(
        get(&mut fixture, session, pane_args(&left_token, "left")),
        failure("no agent in this pane")
    );
    // the split's own block: refused while the left pane holds its block, so free the left first
    status(&mut fixture, session, "idle", "left");
    status(&mut fixture, session, "blocked", "right");
    let agent = agent_of(&get(&mut fixture, session, right()));
    assert_eq!(agent["pane"], "right");
    assert_eq!(agent["approval"]["detail"], "ls");
}

#[test]
fn the_other_panes_block_hides_a_candidate_without_ending_it() {
    let (mut fixture, session, left_token) = one_pane();
    let _ = split(&mut fixture, session);
    let left = || pane_args(&left_token, "left");
    assert!(send_report(&mut fixture, session, left(), start(CLAUDE_A, "aaaa")).ok);
    assert!(send_report(&mut fixture, session, left(), permission("Bash", "ls")).ok);
    status(&mut fixture, session, "blocked", "left");
    assert_eq!(
        agent_of(&get(&mut fixture, session, left()))["approval"]["detail"],
        "ls"
    );
    // the split's agent blocks too: its block takes the session's one status over, then clears
    status(&mut fixture, session, "blocked", "right");
    assert_eq!(agent_of(&get(&mut fixture, session, left())).get("approval"), None);
    status(&mut fixture, session, "active", "right");
    let _ = fixture.core.commit();
    // the left dialog, still open, blocks again
    status(&mut fixture, session, "blocked", "left");
    assert_eq!(
        agent_of(&get(&mut fixture, session, left()))["approval"]["detail"],
        "ls"
    );
}

#[test]
fn an_ask_user_question_makes_no_approval_beside_its_question() {
    let (mut fixture, session, token) = one_pane();
    let left = || pane_args(&token, "left");
    assert!(send_report(&mut fixture, session, left(), start(CLAUDE_A, "aaaa")).ok);
    assert!(send_report(&mut fixture, session, left(), permission("AskUserQuestion", "")).ok);
    status(&mut fixture, session, "blocked", "left");
    assert_eq!(agent_of(&get(&mut fixture, session, left())).get("approval"), None);
}

#[test]
fn a_record_follows_its_pane_identity_through_a_swap() {
    let (mut fixture, session, token) = one_pane();
    let right = split(&mut fixture, session);
    // a swap moves the terminals: both are up
    let panes = fixture
        .store_mut()
        .and_then(|store| store.session_mut(session))
        .expect("the session");
    let _ = panes
        .surface
        .replace(Box::new(FakeSurface::new(&token).with_realized(true)));
    let _ = panes
        .split_surface
        .replace(Box::new(FakeSurface::new(&right).with_realized(true)));
    assert!(
        send_report(
            &mut fixture,
            session,
            pane_args(&token, "left"),
            start(CLAUDE_A, "aaaa")
        )
        .ok
    );
    let response = fixture.session(SessionCmd::Swap, Some(&wire(session)), Args::default());
    assert!(response.ok, "{response:?}");
    // the hook's environment still says `left`: the token wins
    let agent = agent_of(&get(&mut fixture, session, pane_args(&token, "left")));
    assert_eq!(agent["pane"], "right");
    let by_role = agent_of(&get(&mut fixture, session, args(|a| a.pane = Some("right".into()))));
    assert_eq!(by_role, agent);
}

#[test]
fn an_unknown_pane_or_session_is_refused() {
    let (mut fixture, session, token) = one_pane();
    let report = || start(CLAUDE_A, "aaaa");
    let other = Fixture::new(0);
    let foreign = wire(
        other
            .store()
            .session(other.sessions[0])
            .expect("a session")
            .pane_identity,
    );
    for (pane, error) in [
        (args(|a| a.pane_id = Some("nope".into())), "unknown pane id: nope"),
        (
            args(|a| a.pane_id = Some(foreign.clone())),
            &format!("unknown pane id: {foreign}"),
        ),
        (
            args(|a| a.pane = Some("middle".into())),
            "--pane must be left, right, or scratch",
        ),
        (
            args(|a| a.pane = Some("scratch".into())),
            "an agent record is kept for the left or right pane",
        ),
    ] {
        assert_eq!(
            send_report(&mut fixture, session, pane.clone(), report()),
            failure(error)
        );
        assert_eq!(get(&mut fixture, session, pane), failure(error));
    }
    // an unknown token beside a role falls back to the role
    assert!(send_report(&mut fixture, session, pane_args("nope", "left"), report()).ok);
    assert!(get(&mut fixture, session, pane_args(&token, "left")).ok);
    let mut unknown = args(|a| a.pane_id = Some(token.clone()));
    unknown.report = Some(Box::new(report()));
    assert_eq!(
        fixture.send(Command::Agent(AgentCmd::Report), Some(UNKNOWN), unknown),
        failure(&format!("no such session: {UNKNOWN}"))
    );
}

#[test]
fn a_report_is_checked_before_it_is_kept() {
    let (mut fixture, session, token) = one_pane();
    let left = || pane_args(&token, "left");
    let no_report = fixture.send(Command::Agent(AgentCmd::Report), Some(&wire(session)), left());
    assert_eq!(no_report, failure("agent.report requires a report"));
    for (report, error) in [
        (
            conversation("gemini", "SessionStart", CLAUDE_A, "aaaa"),
            "invalid agent: gemini (claude|codex)",
        ),
        (
            conversation("claude", "Stop", CLAUDE_A, "aaaa"),
            "unknown agent event: Stop (SessionStart|UserPromptSubmit|PermissionRequest|SessionEnd)",
        ),
        (
            conversation("claude", "SessionStart", "/etc/passwd.jsonl", "aaaa"),
            "transcript path is outside a Claude projects/ tree: /etc/passwd.jsonl",
        ),
        (
            conversation("codex", "SessionStart", CLAUDE_A, "aaaa"),
            &format!("transcript path is outside a Codex sessions/ tree: {CLAUDE_A}"),
        ),
        (
            AgentReport {
                conversation_id: None,
                ..start(CLAUDE_A, "aaaa")
            },
            "agent.report SessionStart requires transcriptPath and conversationId",
        ),
        (
            AgentReport {
                approval: None,
                ..permission("Bash", "ls")
            },
            "agent.report PermissionRequest requires an approval",
        ),
        // a subagent's prompt names no conversation: it needs the pane's record
        (
            AgentReport {
                transcript_path: None,
                conversation_id: None,
                ..permission("Bash", "ls")
            },
            "no agent in this pane",
        ),
        (
            AgentReport {
                conversation_id: None,
                ..end("aaaa")
            },
            "agent.report SessionEnd requires a conversationId",
        ),
    ] {
        assert_eq!(send_report(&mut fixture, session, left(), report), failure(error));
    }
    assert_eq!(fixture.core.agent_record_count(), 0);
}

#[test]
fn a_record_is_dropped_once_its_pane_identity_leaves_the_model() {
    let mut fixture = Fixture::new(1);
    let (kept, closed) = (fixture.sessions[0], fixture.sessions[1]);
    let right = split(&mut fixture, kept);
    let closed_token = wire(fixture.store().session(closed).expect("a session").pane_identity);
    assert!(send_report(&mut fixture, kept, pane_args(&right, "right"), start(CLAUDE_A, "aaaa")).ok);
    assert!(
        send_report(
            &mut fixture,
            closed,
            pane_args(&closed_token, "left"),
            start(CLAUDE_B, "bbbb")
        )
        .ok
    );
    let _ = fixture.core.commit();
    assert_eq!(fixture.core.agent_record_count(), 2);
    let response = fixture.session(SessionCmd::SplitClose, Some(&wire(kept)), Args::default());
    assert!(response.ok, "{response:?}");
    let response = fixture.session(SessionCmd::Close, Some(&wire(closed)), Args::default());
    assert!(response.ok, "{response:?}");
    let _ = fixture.core.commit();
    assert_eq!(fixture.core.agent_record_count(), 0);
}

#[test]
fn an_undoable_close_keeps_the_record_until_it_is_final() {
    let mut fixture = Fixture::new(1);
    let closing = fixture.sessions[1];
    let token = wire(fixture.store().session(closing).expect("a session").pane_identity);
    assert!(
        send_report(
            &mut fixture,
            closing,
            pane_args(&token, "left"),
            start(CLAUDE_A, "aaaa")
        )
        .ok
    );
    let soft = args(|a| a.undoable = Some(true));
    let response = fixture.session(SessionCmd::Close, Some(&wire(closing)), soft.clone());
    assert!(response.ok, "{response:?}");
    assert!(fixture.store().pending_close_session(closing).is_some(), "a soft close");
    let _ = fixture.core.commit();
    assert_eq!(fixture.core.agent_record_count(), 1, "the grace still holds the pane");
    assert!(fixture.store_mut().unwrap().undo_pending_close(None));
    let _ = fixture.core.commit();
    let agent = agent_of(&get(&mut fixture, closing, pane_args(&token, "left")));
    assert_eq!(agent["conversationId"], "aaaa");
    let response = fixture.session(SessionCmd::Close, Some(&wire(closing)), soft);
    assert!(response.ok, "{response:?}");
    assert!(fixture.store_mut().unwrap().finalize_pending_close_of_session(closing));
    let _ = fixture.core.commit();
    assert_eq!(fixture.core.agent_record_count(), 0, "a final close drops it");
}

#[test]
fn the_record_never_reaches_the_snapshot_or_the_tree() {
    let dir = short_tempdir();
    let mut persistence = LibraryPersistence::new(dir.path());
    let library = persistence.bootstrap("/home/u");
    let mut core = Core::new(
        CoreConfig {
            app: ngterm_proto::AppIdentity::from_recorded("0.1.0", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            settings: AppSettings::default(),
            persistence: Some(persistence),
        },
    );
    let session = core.library().all_open_sessions()[0].id();
    let token = wire(core.library().all_open_sessions()[0].pane_identity);
    let mut request = pane_args(&token, "left");
    request.report = Some(Box::new(conversation(
        "claude",
        "SessionStart",
        CLAUDE_A,
        "conversation-marker",
    )));
    assert!(
        send(
            &mut core,
            Command::Agent(AgentCmd::Report),
            Some(&wire(session)),
            request
        )
        .ok
    );
    let _ = core.commit();
    assert!(core.save_all());
    let tree = send(&mut core, Command::Tree, None, Args::default());
    assert!(!format!("{tree:?}").contains("conversation-marker"));
    let mut files = vec![dir.path().to_path_buf()];
    let mut read = 0;
    while let Some(path) = files.pop() {
        if path.is_dir() {
            files.extend(std::fs::read_dir(&path).unwrap().map(|entry| entry.unwrap().path()));
        } else {
            let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
            assert!(!text.contains("conversation-marker"), "{}", path.display());
            assert!(!text.contains("aaaa.jsonl"), "{}", path.display());
            read += 1;
        }
    }
    assert!(read > 0, "the snapshot wrote nothing");
}

// MARK: - the lazy foreground check

fn stat(pid: i32, parent: i32, pgrp: i32, session: i32) -> ProcessStat {
    ProcessStat {
        pid,
        ppid: parent,
        pgrp,
        session,
    }
}

/// The fixture's pane running `claude` from a shell (`sh`, pid 100, the session leader; `claude`, group 101), its
/// terminal's probe and its token.
fn agent_in_a_shell() -> (Fixture, Uuid, String, FakeSurfaceProbe) {
    let (mut fixture, session, token) = one_pane();
    let table = FakeProcessTable::default();
    table.insert(stat(100, 1, 100, 100), Some(&["sh"]));
    table.insert(stat(101, 100, 101, 100), Some(&["claude", "--model", "sonnet"]));
    fixture.core.set_process_table(Box::new(table), Some("sh".to_owned()));
    let surface = FakeSurface::new(&token);
    surface.probe().set_realized(true);
    surface.probe().set_foreground_pid(Some(101));
    let probe = surface.probe();
    let slot = fixture
        .store_mut()
        .and_then(|store| store.session_mut(session))
        .map(|session| &mut session.surface)
        .expect("the session");
    let _ = slot.replace(Box::new(surface));
    (fixture, session, token, probe)
}

#[test]
fn a_pane_back_at_its_shell_clears_the_record() {
    let (mut fixture, session, token, probe) = agent_in_a_shell();
    let left = || pane_args(&token, "left");
    assert!(send_report(&mut fixture, session, left(), start(CLAUDE_A, "aaaa")).ok);
    assert!(get(&mut fixture, session, left()).ok);
    // a foreground that cannot be read keeps it
    probe.set_foreground_pid(None);
    assert!(get(&mut fixture, session, left()).ok);
    // the agent exited: the shell holds the foreground again
    probe.set_foreground_pid(Some(100));
    assert_eq!(get(&mut fixture, session, left()), failure("no agent in this pane"));
    assert_eq!(fixture.core.agent_record_count(), 0);
    // and it stays cleared once the next program runs, until a hook reports again
    probe.set_foreground_pid(Some(101));
    assert_eq!(get(&mut fixture, session, left()), failure("no agent in this pane"));
}

#[test]
fn a_powershell_without_job_control_settles_nothing_on_linux_and_macos() {
    use crate::command_restore::PaneForeground;
    use crate::commands::agent::agent_exited;
    let shell = |name: &str| PaneForeground::ForegroundShell(name.to_owned());
    for windows in [false, true] {
        assert!(agent_exited(Some(&shell("zsh")), windows));
        assert_eq!(agent_exited(Some(&shell("pwsh")), windows), windows);
        assert_eq!(agent_exited(Some(&shell("powershell")), windows), windows);
        assert!(!agent_exited(
            Some(&PaneForeground::Program(vec!["claude".to_owned()])),
            windows
        ));
        assert!(!agent_exited(None, windows));
    }
    // `claude` started from an interactive PowerShell, which leads the group it runs in
    let (mut fixture, session, token, probe) = agent_in_a_shell();
    let table = FakeProcessTable::default();
    table.insert(stat(100, 1, 100, 100), Some(&["pwsh", "-NoLogo"]));
    table.insert(stat(101, 100, 100, 100), Some(&["claude"]));
    fixture.core.set_process_table(Box::new(table), Some("pwsh".to_owned()));
    probe.set_foreground_pid(Some(100));
    let left = || pane_args(&token, "left");
    assert!(send_report(&mut fixture, session, left(), start(CLAUDE_A, "aaaa")).ok);
    assert_eq!(get(&mut fixture, session, left()).ok, !cfg!(windows));
}
