use serde_json::json;
use uuid::Uuid;

use super::*;

const HOST: &str = "studio";
const CREATED: &str = "0E2B7C1A-5D4F-4E3B-9A61-7C2D8F0B1A23";

fn reply(body: &serde_json::Value) -> RemoteCommandResult {
    answered(0, &body.to_string(), "")
}

fn answered(status: i32, stdout: &str, stderr: &str) -> RemoteCommandResult {
    RemoteCommandResult {
        status,
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    }
}

/// ssh lost the connection before the far side printed anything.
fn lost() -> RemoteCommandResult {
    answered(255, "", "ssh: connection lost\n")
}

/// A CLI older than the line it was given.
fn usage() -> RemoteCommandResult {
    answered(64, "", "Error: Unknown option '--no-select'\n")
}

fn restore_mode(active: &str) -> RemoteCommandResult {
    reply(&json!({"ok": true, "result": {"restore": {
        "configured": active, "requestedAtLaunch": active, "active": active, "restartRequired": false,
    }}}))
}

/// One pane session `number`, with the far side's `parent` and `content` when it has them.
fn session(number: u128, parent: Option<u128>, content: Option<&str>) -> serde_json::Value {
    let id = Uuid::from_u128(number).to_string().to_uppercase();
    let daemon = crate::live::daemon_name(Uuid::from_u128(0x1000 + number));
    let mut session = json!({
        "id": id, "name": format!("s{number}"), "windowID": "w1", "windowName": "main", "workspaceID": "ws1",
        "workspaceName": "work", "cwd": "/repo", "panes": [{"pane": "left", "daemon": daemon}],
    });
    if let Some(parent) = parent {
        session["parent"] = json!(Uuid::from_u128(parent).to_string().to_uppercase());
    }
    if let Some(content) = content {
        session["content"] = json!(content);
        session["panes"] = json!([]);
    }
    session
}

fn created_session() -> serde_json::Value {
    let mut created = session(0xc1, None, None);
    created["id"] = json!(CREATED);
    created
}

fn listing(sessions: &[serde_json::Value]) -> RemoteCommandResult {
    reply(&json!({"ok": true, "result": {"remote": {
        "endpoint": {"executable": "/ngterm-session", "socketDirectory": "/tmp/ngterm-live-x"},
        "sessions": sessions,
    }}}))
}

fn names(sessions: &[&ControlRemoteSession]) -> Vec<String> {
    sessions.iter().map(|session| session.name.clone()).collect()
}

// MARK: - replies and their strings

#[test]
fn reply_of_reads_a_trimmed_receipt_and_nothing_else() {
    let receipt = answered(0, "  {\"ok\": true, \"result\": {}}\n", "");
    assert!(reply_of(&receipt).is_some_and(|reply| reply.ok));
    assert!(reply_of(&answered(0, "not json", "")).is_none());
    assert!(reply_of(&lost()).is_none());
}

#[test]
fn no_answer_names_what_ssh_said_only_when_it_said_something() {
    assert_eq!(
        no_answer(&lost(), HOST, "nothing was created"),
        "studio gave no answer (ssh: connection lost); nothing was created"
    );
    assert_eq!(
        no_answer(&answered(-1, "", "  \n"), HOST, "nothing was created"),
        "studio gave no answer; nothing was created"
    );
}

#[test]
fn too_old_names_the_host() {
    assert_eq!(
        too_old(HOST),
        "the ngterm on studio is too old for this; nothing was created"
    );
}

#[test]
fn not_live_accepts_only_the_live_restore_mode() {
    assert_eq!(not_live(&restore_mode("live"), HOST), None);
    let refusals = [
        (
            restore_mode("none"),
            "studio does not run live sessions (restore mode none), so a session made there could not attach; \
             nothing was created",
        ),
        (usage(), "the ngterm on studio is too old for this; nothing was created"),
        (
            reply(&json!({"ok": false, "error": "ngterm is stopped"})),
            "ngterm is stopped; nothing was created",
        ),
        (
            reply(&json!({"ok": false})),
            "the remote command failed on studio; nothing was created",
        ),
        (
            lost(),
            "studio gave no answer (ssh: connection lost); nothing was created",
        ),
        (
            reply(&json!({"ok": true, "result": {}})),
            "studio did not name its restore mode; nothing was created",
        ),
    ];
    for (result, shown) in refusals {
        assert_eq!(not_live(&result, HOST).as_deref(), Some(shown));
        assert_eq!(check_restore_mode(&result, HOST), Err(shown.to_owned()));
    }
    assert_eq!(check_restore_mode(&restore_mode("live"), HOST), Ok(()));
}

#[test]
fn end_failure_is_none_once_the_origin_says_it_ended() {
    assert_eq!(end_failure(&reply(&json!({"ok": true})), HOST), None);
    // a full receipt counts whatever the transport did afterwards
    assert_eq!(end_failure(&answered(255, "{\"ok\": true}", "broken pipe"), HOST), None);
    assert_eq!(
        end_failure(&reply(&json!({"ok": false, "error": "no session matches"})), HOST).as_deref(),
        Some("no session matches")
    );
    assert_eq!(
        end_failure(&reply(&json!({"ok": false})), HOST).as_deref(),
        Some("the remote command failed on studio")
    );
    assert_eq!(
        end_failure(&lost(), HOST).as_deref(),
        Some("studio gave no answer (ssh: connection lost); whether the session ended there is unknown")
    );
}

#[test]
fn origin_created_names_the_created_id_or_why_there_is_none() {
    let receipt = json!({"ok": true, "result": {"id": CREATED}});
    assert_eq!(
        origin_created(&reply(&receipt), HOST, "companion"),
        Ok(CREATED.to_owned())
    );
    let failures = [
        (
            lost(),
            "studio gave no answer (ssh: connection lost); whether the companion was created there is unknown"
                .to_owned(),
        ),
        (
            answered(255, &receipt.to_string(), "broken pipe"),
            format!(
                "created companion {CREATED} on studio but the connection failed before attaching it; `ngtermctl \
                 live attach studio {CREATED}` attaches it"
            ),
        ),
        (
            reply(&json!({"ok": false, "error": "no window to create in"})),
            "no window to create in".to_owned(),
        ),
        (
            answered(1, "{\"ok\": true, \"result\": {}}", ""),
            "the remote command failed on studio".to_owned(),
        ),
        (
            reply(&json!({"ok": true, "result": {"id": "not-a-uuid"}})),
            "studio created a companion but did not name it".to_owned(),
        ),
    ];
    for (result, shown) in failures {
        assert_eq!(origin_created(&result, HOST, "companion"), Err(shown));
    }
}

#[test]
fn session_created_is_too_old_on_a_refused_line_and_names_a_session_otherwise() {
    let receipt = reply(&json!({"ok": true, "result": {"id": CREATED}}));
    assert_eq!(session_created(&receipt, HOST), Ok(CREATED.to_owned()));
    assert_eq!(session_created(&usage(), HOST), Err(too_old(HOST)));
    assert_eq!(
        session_created(&lost(), HOST),
        Err(
            "studio gave no answer (ssh: connection lost); whether the session was created there is unknown".to_owned()
        )
    );
}

#[test]
fn unattached_names_the_line_that_attaches_it() {
    assert_eq!(
        unattached("session", CREATED, HOST, NOT_ATTACHABLE),
        format!(
            "created session {CREATED} on studio but could not attach it: it did not become attachable; `ngtermctl \
             live attach studio {CREATED}` attaches it"
        )
    );
}

#[test]
fn the_create_lines_are_the_origins_own() {
    assert_eq!(RESTORE_MODE_ARGUMENTS, ["restore", "mode"]);
    assert_eq!(NEW_SESSION_ARGUMENTS, ["session", "new", "--no-select"]);
    assert_eq!(
        end_session_arguments(CREATED),
        ["session", "close", "--target", CREATED]
    );
}

// MARK: - listings

#[test]
fn read_remote_tree_stamps_the_host_on_a_listing() {
    let tree = read_remote_tree(&listing(&[created_session()]), HOST).unwrap();
    assert_eq!(tree.sessions.len(), 1);
    assert_eq!(tree.sessions[0].id, CREATED);
    assert_eq!(tree.host.as_deref(), Some(HOST));
}

/// An agterm Mac's `agtermctl zmx tree --json`, in the shape upstream's own suite pins
/// (`agtermTests/ControlServerRemoteGroupTests.swift`'s `tree`, `ControlServerZmxTests.swift`), after the listing's
/// marker line.
const AGTERM_LISTING: &str = concat!(
    "ngterm-peer: agterm\n",
    r#"{"ok":true,"result":{"remote":{"endpoint":{"executable":"/Applications/agterm.app/zmx","#,
    r#""socketDirectory":"/tmp/agterm-zmx-abc"},"presentation":1,"sessions":[{"id":"P","name":"build","#,
    r#""windowID":"w1","windowName":"main","workspaceID":"ws1","workspaceName":"work","cwd":"/repo","#,
    r#""splitAxis":"leftRight","status":"working","statusColor":"green","companions":[{"id":"C","name":"tests"}],"#,
    r#""panes":[{"pane":"left","daemon":"agterm-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1"},"#,
    r#"{"pane":"right","daemon":"agterm-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa2","foreground":["claude"]}]},"#,
    r#"{"id":"C","name":"tests","windowID":"w1","windowName":"main","workspaceID":"ws1","workspaceName":"work","#,
    r#""cwd":"/repo","parent":"P","panes":[{"pane":"left","daemon":"agterm-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa3"}]}]}}}"#,
    "\n"
);

#[test]
fn an_agterm_listing_reads_whole_with_its_peer_stamped_and_learned() {
    let host = "origin-replies-agterm.example";
    let tree = read_remote_tree(&answered(0, AGTERM_LISTING, ""), host).unwrap();
    assert_eq!(tree.host.as_deref(), Some(host));
    assert_eq!(tree.endpoint.peer, Some(crate::remote::peer::RemotePeer::Agterm));
    assert_eq!(tree.endpoint.executable, "/Applications/agterm.app/zmx");
    assert_eq!(tree.presentation, Some(1));
    assert_eq!(names(&pickable_sessions(&tree)), ["build", "tests"]);
    assert_eq!(
        tree.sessions[0].panes[1].daemon,
        "agterm-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa2"
    );
    assert_eq!(
        crate::remote::peer::peer_of(host),
        crate::remote::peer::RemotePeer::Agterm
    );
    // the same host answering as ngterm later goes back to ngterm's lines
    read_remote_tree(&listing(&[created_session()]), host).unwrap();
    assert_eq!(
        crate::remote::peer::peer_of(host),
        crate::remote::peer::RemotePeer::Ngterm
    );
}

/// A listing pinned to one app reads the same, but leaves which app the host's own listing finds as it was.
#[test]
fn a_pinned_listing_reads_whole_and_learns_nothing() {
    let host = "origin-replies-pinned.example";
    read_remote_tree(&listing(&[created_session()]), host).unwrap();
    let tree = read_peer_tree(&answered(0, AGTERM_LISTING, ""), host).unwrap();
    assert_eq!(tree.host.as_deref(), Some(host));
    assert_eq!(tree.endpoint.peer, Some(crate::remote::peer::RemotePeer::Agterm));
    assert_eq!(
        crate::remote::peer::peer_of(host),
        crate::remote::peer::RemotePeer::Ngterm
    );
    assert_eq!(
        read_peer_tree(&answered(1, "", ""), host).unwrap_err(),
        format!("the remote command failed on {host}")
    );
}

#[test]
fn only_the_listings_marker_says_agterm_never_the_answer_itself() {
    let claims = json!({"ok": true, "result": {"remote": {
        "endpoint": {"executable": "/opt/ngterm-session", "socketDirectory": "/tmp/n", "peer": "agterm"},
        "sessions": []}}});
    let tree = read_remote_tree(&reply(&claims), HOST).unwrap();
    assert_eq!(tree.endpoint.peer, None);
}

#[test]
fn an_agterm_refusal_under_the_marker_is_read_as_its_own_words() {
    let refusal = "ngterm-peer: agterm\n{\"ok\":false,\"error\":\"agterm is not running\"}\n";
    assert_eq!(
        read_remote_tree(&answered(1, refusal, ""), HOST).unwrap_err(),
        "agterm is not running"
    );
}

#[test]
fn read_remote_tree_prefers_the_far_refusal_then_ssh_then_a_generic_failure() {
    let refusal = answered(1, "{\"ok\": false, \"error\": \"live sessions are off\"}", "ignored");
    assert_eq!(read_remote_tree(&refusal, HOST).unwrap_err(), "live sessions are off");
    assert_eq!(read_remote_tree(&lost(), HOST).unwrap_err(), "ssh: connection lost");
    assert_eq!(
        read_remote_tree(&answered(1, "", ""), HOST).unwrap_err(),
        "the remote command failed on studio"
    );
    // an ok-looking payload from a nonzero process is never accepted
    let mut late = listing(&[created_session()]);
    late.status = 255;
    assert!(read_remote_tree(&late, HOST).is_err());
    assert!(read_remote_tree(&answered(0, "garbage", ""), HOST).is_err());
}

#[test]
fn the_picker_leaves_page_companions_out_and_lists_parents_first() {
    let tree = read_remote_tree(
        &listing(&[
            session(2, Some(1), None),
            session(1, None, None),
            session(3, Some(1), Some("page")),
            session(4, None, None),
        ]),
        HOST,
    )
    .unwrap();
    assert!(
        tree.sessions
            .iter()
            .filter(|session| !is_pickable(session))
            .all(|session| session.name == "s3")
    );
    assert_eq!(names(&pickable_sessions(&tree)), ["s1", "s4", "s2"]);
}

// MARK: - the wait until the created session is listed

#[test]
fn the_wait_lists_at_once_within_the_tree_deadline() {
    let now = Instant::now();
    let wait = ListingWait::new(CREATED.to_owned(), now);
    assert_eq!(wait.created(), CREATED);
    assert_eq!(wait.next(now), WaitStep::List(TREE_DEADLINE));
    // near the end of the budget a listing gets only what is left
    let late = now + Duration::from_secs(17);
    assert_eq!(wait.next(late), WaitStep::List(Duration::from_secs(3)));
}

#[test]
fn a_listing_without_the_session_pauses_before_the_next() {
    let now = Instant::now();
    let mut wait = ListingWait::new(CREATED.to_owned(), now);
    assert!(wait.listed(&listing(&[]), HOST, now).is_none());
    assert_eq!(wait.next(now), WaitStep::Wait);
    assert_eq!(wait.next(now + Duration::from_millis(999)), WaitStep::Wait);
    assert!(matches!(wait.next(now + PAUSE), WaitStep::List(_)));
    let (session, tree) = wait.listed(&listing(&[created_session()]), HOST, now + PAUSE).unwrap();
    assert_eq!(session.id, CREATED);
    assert_eq!(tree.host.as_deref(), Some(HOST));
}

#[test]
fn a_spent_budget_gives_up_with_the_last_reason() {
    let now = Instant::now();
    let mut wait = ListingWait::new(CREATED.to_owned(), now);
    let end = now + READY_BUDGET;
    assert_eq!(wait.next(end), WaitStep::GiveUp(NOT_ATTACHABLE.to_owned()));
    assert!(wait.listed(&lost(), HOST, now).is_none());
    assert_eq!(wait.next(end), WaitStep::GiveUp("ssh: connection lost".to_owned()));
    // a listing that answers without the session keeps the reason a failed one gave
    assert!(wait.listed(&listing(&[]), HOST, now + PAUSE).is_none());
    assert_eq!(wait.next(end), WaitStep::GiveUp("ssh: connection lost".to_owned()));
    assert_eq!(
        wait.unattached(HOST, "ssh: connection lost"),
        unattached(SESSION_NOUN, CREATED, HOST, "ssh: connection lost")
    );
}
