use std::sync::Mutex;

use serde_json::json;

use super::*;
use crate::remote::peer::AGTERM_ANSWER;

fn answered(status: i32, stdout: &str) -> RemoteCommandResult {
    RemoteCommandResult {
        status,
        stdout: stdout.to_owned(),
        stderr: String::new(),
    }
}

fn session(id: &str) -> serde_json::Value {
    json!({
        "id": id, "name": id, "windowID": "w", "windowName": "main", "workspaceID": "ws", "workspaceName": "work",
        "cwd": "/repo", "panes": [{"pane": "left", "daemon": format!("agterm-{:032x}", 7)}]
    })
}

fn answer(executable: &str, sessions: &[serde_json::Value]) -> String {
    json!({"ok": true, "result": {"remote": {
        "endpoint": {"executable": executable, "socketDirectory": "/tmp/s"},
        "sessions": sessions,
    }}})
    .to_string()
}

/// A Mac running agterm alone, listing `sessions`.
fn agterm_alone(sessions: &[serde_json::Value]) -> RemoteCommandResult {
    answered(0, &format!("{AGTERM_ANSWER}\n{}\n", answer("/zmx", sessions)))
}

/// A Mac running both apps: ngterm listing `own`, agterm `theirs`.
fn both(own: &[serde_json::Value], theirs: &[serde_json::Value]) -> RemoteCommandResult {
    answered(
        0,
        &format!(
            "{}\n{AGTERM_ANSWER}\n{}\n",
            answer("/ngterm-session", own),
            answer("/zmx", theirs)
        ),
    )
}

fn mode(configured: &str, active: &str, reason: Option<&str>) -> RemoteCommandResult {
    let mut restore = json!({
        "configured": configured, "requestedAtLaunch": active, "active": active,
        "restartRequired": configured != active
    });
    if let Some(reason) = reason {
        restore["unavailableReason"] = json!(reason);
    }
    answered(0, &format!("{}\n", json!({"ok": true, "result": {"restore": restore}})))
}

fn listed(listing: &RemoteCommandResult, restore: &RemoteCommandResult, host: &str) -> ControlRemoteTree {
    tree::decode(&with_restore(listing.clone(), restore).stdout)
        .unwrap()
        .stamped(host)
}

const OFF: &str = "agterm on studio is not in Live sessions mode: set Settings > General > Restore sessions to Live \
                   sessions and restart agterm; only sessions started after that can be attached";

#[test]
fn the_restore_line_asks_agterms_own_cli_for_its_mode() {
    let tail = restore_tail();
    assert!(tail.contains("agtermctl"), "{tail}");
    assert!(!tail.contains("ngtermctl"), "{tail}");
    for word in ["restore", "mode", "--json"] {
        assert!(tail.contains(&format!("'\\''{word}'\\''")), "{tail}");
    }
}

#[test]
fn only_an_agterm_that_listed_nothing_is_asked_its_mode() {
    assert!(asks_restore_mode(&agterm_alone(&[])));
    assert!(asks_restore_mode(&both(&[session("N1")], &[])));
    assert!(!asks_restore_mode(&agterm_alone(&[session("A1")])));
    assert!(!asks_restore_mode(&both(&[], &[session("A1")])));
    assert!(!asks_restore_mode(&answered(
        0,
        &format!("{}\n", answer("/ngterm-session", &[]))
    )));
    let mut failed = agterm_alone(&[]);
    failed.status = 1;
    assert!(!asks_restore_mode(&failed));
    assert!(!asks_restore_mode(&answered(0, "garbage")));
}

#[test]
fn agterms_mode_goes_on_its_own_part_of_the_listing() {
    let alone = listed(&agterm_alone(&[]), &mode("none", "none", None), "studio");
    assert_eq!(
        alone.restore.as_ref().map(|status| status.active.as_str()),
        Some("none")
    );
    assert!(alone.sessions.is_empty());
    let beside = listed(&both(&[session("N1")], &[]), &mode("none", "none", None), "studio");
    assert_eq!(beside.restore, None);
    assert_eq!(beside.sessions.len(), 1);
    let agterm = beside.beside.as_deref().unwrap();
    assert_eq!(
        agterm.restore.as_ref().map(|status| status.configured.as_str()),
        Some("none")
    );
    assert_eq!(agterm.endpoint.peer(), RemotePeer::Agterm);
}

#[test]
fn a_failed_or_unreadable_answer_leaves_the_listing_as_it_was() {
    let listing = agterm_alone(&[]);
    assert_eq!(with_restore(listing.clone(), &answered(255, "")), listing);
    let unread = listed(&listing, &answered(0, "not json"), "studio");
    assert_eq!(unread.restore, None);
    let refused = listed(&listing, &answered(0, r#"{"ok":false,"error":"no"}"#), "studio");
    assert_eq!(refused.restore, None);
    // a listing that printed no last newline still ends before the line
    let bare = answered(0, listing.stdout.trim_end());
    assert_eq!(
        listed(&bare, &mode("none", "none", None), "studio")
            .restore
            .unwrap()
            .active,
        "none"
    );
}

#[test]
fn the_hint_says_what_to_change_in_agterm_for_each_mode_it_is_in() {
    let off = listed(&agterm_alone(&[]), &mode("none", "none", None), "agterm:studio");
    assert_eq!(hint(&off).as_deref(), Some(OFF));
    let rerun = listed(&agterm_alone(&[]), &mode("rerun", "rerun", None), "studio");
    assert_eq!(hint(&rerun).as_deref(), Some(OFF));
    let unrestarted = listed(&agterm_alone(&[]), &mode("live", "none", None), "studio");
    assert_eq!(
        hint(&unrestarted).as_deref(),
        Some(
            "agterm on studio is not in Live sessions mode until it restarts: restart agterm; only sessions started \
             after that can be attached"
        )
    );
    let refused = listed(
        &agterm_alone(&[]),
        &mode("live", "none", Some("the login shell is fish")),
        "studio",
    );
    assert_eq!(
        hint(&refused).as_deref(),
        Some("agterm on studio could not start Live sessions: the login shell is fish")
    );
    let beside = listed(&both(&[session("N1")], &[]), &mode("none", "none", None), "studio");
    assert_eq!(hint(&beside).as_deref(), Some(OFF));
}

#[test]
fn no_hint_for_agterm_in_live_mode_one_with_sessions_or_a_host_without_agterm() {
    let live = listed(&agterm_alone(&[]), &mode("live", "live", None), "studio");
    assert_eq!(hint(&live), None);
    let unasked = tree::decode(&agterm_alone(&[]).stdout).unwrap();
    assert_eq!(hint(&unasked), None);
    let mut listing = listed(&agterm_alone(&[]), &mode("none", "none", None), "studio");
    listing.sessions = tree::decode(&agterm_alone(&[session("A1")]).stdout).unwrap().sessions;
    assert_eq!(hint(&listing), None);
    let ngterm = tree::decode(&answer("/ngterm-session", &[])).unwrap();
    assert_eq!(hint(&ngterm), None);
    let nameless = tree::decode(&with_restore(agterm_alone(&[]), &mode("none", "none", None)).stdout).unwrap();
    assert!(
        hint(&nameless)
            .unwrap()
            .starts_with("agterm is not in Live sessions mode: ")
    );
}

/// Answers each line in turn, recording what it was given.
struct Lines {
    answers: Mutex<Vec<RemoteCommandResult>>,
    calls: Mutex<Vec<(Vec<String>, Duration)>>,
}

impl Lines {
    fn new(answers: Vec<RemoteCommandResult>) -> Self {
        Self {
            answers: Mutex::new(answers),
            calls: Mutex::default(),
        }
    }

    fn calls(&self) -> Vec<(Vec<String>, Duration)> {
        self.calls.lock().unwrap().clone()
    }
}

impl RemoteCommandRunner for Lines {
    fn run(&self, argv: &[String], deadline: Duration) -> RemoteCommandResult {
        self.calls.lock().unwrap().push((argv.to_vec(), deadline));
        self.answers.lock().unwrap().remove(0)
    }
}

#[test]
fn a_listing_asks_the_mode_in_one_more_ssh_call_only_when_agterm_listed_nothing() {
    let runner = Lines::new(vec![agterm_alone(&[]), mode("none", "none", None)]);
    let result = run_listing(&runner, "agterm:studio", tree::TREE_DEADLINE);
    let calls = runner.calls();
    assert_eq!(calls.len(), 2);
    let (argv, deadline) = &calls[1];
    assert_eq!(*deadline, RESTORE_DEADLINE + Duration::from_secs(10));
    assert_eq!(argv.last(), Some(&restore_tail()));
    assert_eq!(argv[..argv.len() - 1], tree::ssh_arguments("studio", false)[..]);
    let tree = tree::decode(&result.stdout).unwrap().stamped("agterm:studio");
    assert_eq!(hint(&tree).as_deref(), Some(OFF));

    for listing in [
        agterm_alone(&[session("A1")]),
        answered(0, &answer("/ngterm-session", &[])),
    ] {
        let runner = Lines::new(vec![listing.clone()]);
        assert_eq!(run_listing(&runner, "studio", tree::TREE_DEADLINE), listing);
        assert_eq!(runner.calls().len(), 1);
    }
}
