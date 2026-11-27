//! The server's words for each operation, and upstream's drop cases of `RemoteOperationRequestTests`.

use uuid::Uuid;

use super::*;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// Asserts each operation's words and operands.
fn check(cases: Vec<(RemoteOperation, Vec<String>, Vec<String>)>) {
    for (operation, words, operands) in cases {
        assert_eq!(operation.words(), words, "{operation:?}");
        assert_eq!(operation.operands(), operands, "{operation:?}");
    }
}

#[test]
fn new_rows_and_pane_actions_name_the_servers_words() {
    let (s, w, a) = (id(1), id(2), id(3));
    let (s_text, w_text, a_text) = (uuid_string(s), uuid_string(w), uuid_string(a));
    let shell = RemoteShellOptions {
        name: Some("-x".to_owned()),
        cwd: Some("/srv".to_owned()),
        command: Some("htop".to_owned()),
    };
    let cases: Vec<(RemoteOperation, Vec<String>, Vec<String>)> = vec![
        (
            RemoteOperation::NewSession {
                workspace: w,
                anchor: None,
                shell,
            },
            owned(&[
                "session",
                "new",
                "--workspace",
                &w_text,
                "--name=-x",
                "--cwd=/srv",
                "--command=htop",
            ]),
            vec![],
        ),
        (
            RemoteOperation::NewSession {
                workspace: w,
                anchor: Some(RemoteSessionPlacement::Before(a)),
                shell: RemoteShellOptions::default(),
            },
            owned(&["session", "new", "--before", &a_text]),
            vec![],
        ),
        (
            RemoteOperation::NewCompanion {
                parent: s,
                shell: RemoteShellOptions::default(),
            },
            owned(&["session", "new", "--parent", &s_text]),
            vec![],
        ),
        (
            RemoteOperation::SetSplit {
                session: s,
                shown: true,
                axis: Some("horizontal".to_owned()),
            },
            owned(&[
                "session",
                "split",
                "visibility",
                "on",
                "--target",
                &s_text,
                "--axis",
                "horizontal",
            ]),
            vec![],
        ),
        (
            RemoteOperation::SetScratch {
                session: s,
                shown: false,
                command: None,
            },
            owned(&["session", "scratch", "off", "--target", &s_text]),
            vec![],
        ),
        (
            RemoteOperation::CloseSplit(s),
            owned(&["session", "split", "close", "--target", &s_text]),
            vec![],
        ),
    ];
    check(cases);
}

#[test]
fn renames_moves_closes_and_workspace_actions_name_the_servers_words() {
    let (s, w, a) = (id(1), id(2), id(3));
    let (s_text, w_text, a_text) = (uuid_string(s), uuid_string(w), uuid_string(a));
    let cases: Vec<(RemoteOperation, Vec<String>, Vec<String>)> = vec![
        (
            RemoteOperation::Swap(s),
            owned(&["session", "swap", "--target", &s_text]),
            vec![],
        ),
        (
            RemoteOperation::RenameSession {
                session: s,
                name: "-build".to_owned(),
            },
            owned(&["session", "rename", "--target", &s_text]),
            owned(&["-build"]),
        ),
        (
            RemoteOperation::MoveSession {
                session: s,
                to: RemoteSessionPlacement::Workspace(w),
            },
            owned(&["session", "move", "--target", &s_text]),
            owned(&[&w_text]),
        ),
        (
            RemoteOperation::MoveSession {
                session: s,
                to: RemoteSessionPlacement::After(a),
            },
            owned(&["session", "move", "--target", &s_text, "--after", &a_text]),
            vec![],
        ),
        (
            RemoteOperation::MoveSession {
                session: s,
                to: RemoteSessionPlacement::Step("top".to_owned()),
            },
            owned(&["session", "move", "--target", &s_text, "--to", "top"]),
            vec![],
        ),
        (
            RemoteOperation::CloseSession(s),
            owned(&["session", "close", "--target", &s_text]),
            vec![],
        ),
        (
            RemoteOperation::NewWorkspace {
                name: Some("dev".to_owned()),
            },
            owned(&["workspace", "new"]),
            owned(&["dev"]),
        ),
        (
            RemoteOperation::RenameWorkspace {
                workspace: w,
                name: "ops".to_owned(),
            },
            owned(&["workspace", "rename", "--target", &w_text]),
            owned(&["ops"]),
        ),
        (
            RemoteOperation::DeleteWorkspace(w),
            owned(&["workspace", "delete", "--target", &w_text]),
            vec![],
        ),
        (
            RemoteOperation::MoveWorkspace {
                workspace: w,
                to: "up".to_owned(),
            },
            owned(&["workspace", "move", "--target", &w_text, "--to", "up"]),
            vec![],
        ),
    ];
    check(cases);
}

#[test]
fn a_move_before_an_anchor_names_it() {
    let (s, a) = (id(1), id(3));
    let operation = RemoteOperation::MoveSession {
        session: s,
        to: RemoteSessionPlacement::Before(a),
    };
    let words = owned(&[
        "session",
        "move",
        "--target",
        &uuid_string(s),
        "--before",
        &uuid_string(a),
    ]);
    check(vec![(operation, words, vec![])]);
}

#[test]
fn labels_name_the_action_and_its_direction() {
    let s = id(1);
    let split = |shown| RemoteOperation::SetSplit {
        session: s,
        shown,
        axis: None,
    };
    let scratch = |shown| RemoteOperation::SetScratch {
        session: s,
        shown,
        command: None,
    };
    assert_eq!(split(true).label(), "Show Split");
    assert_eq!(split(false).label(), "Hide Split");
    assert_eq!(scratch(true).label(), "Show Scratch");
    assert_eq!(scratch(false).label(), "Hide Scratch");
    assert_eq!(RemoteOperation::new_session(s).label(), "New Session");
    assert_eq!(RemoteOperation::NewWorkspace { name: None }.target(), None);
}

#[test]
fn a_repeat_is_the_same_action_on_the_same_row() {
    let (a, b) = (id(1), id(2));
    let rename = |session| RemoteOperation::RenameSession {
        session,
        name: "x".to_owned(),
    };
    assert_eq!(rename(a).key(), rename(a).key());
    assert_ne!(rename(a).key(), rename(b).key());
    assert_ne!(rename(a).key(), RemoteOperation::Swap(a).key());
}

#[test]
fn only_a_command_replaces_the_scratch_and_only_pane_actions_pin_panes() {
    let s = id(1);
    let scratch = |command: Option<&str>| RemoteOperation::SetScratch {
        session: s,
        shown: true,
        command: command.map(str::to_owned),
    };
    assert!(scratch(Some("htop")).replaces_scratch());
    assert!(!scratch(Some("")).replaces_scratch());
    assert!(!scratch(None).replaces_scratch());
    assert!(RemoteOperation::Swap(s).pins_panes());
    assert!(RemoteOperation::CloseSplit(s).pins_panes());
    assert!(!RemoteOperation::CloseSession(s).pins_panes());
}

#[test]
fn a_session_drop_becomes_an_anchor_among_the_other_rows() {
    let ids = [id(1), id(2), id(3)];
    let target = id(9);
    assert_eq!(
        RemoteOperation::drop_placement(ids[0], target, &ids, 1),
        RemoteSessionPlacement::Before(ids[2])
    );
    assert_eq!(
        RemoteOperation::drop_placement(ids[2], target, &ids, 0),
        RemoteSessionPlacement::Before(ids[0])
    );
    assert_eq!(
        RemoteOperation::drop_placement(ids[0], target, &ids, 2),
        RemoteSessionPlacement::After(ids[2])
    );
    assert_eq!(
        RemoteOperation::drop_placement(ids[0], target, &[], 0),
        RemoteSessionPlacement::Workspace(target)
    );
}

#[test]
fn a_workspace_drop_is_one_move_step_or_nothing() {
    let cases = [
        (2, 0, Some("top")),
        (1, 4, Some("bottom")),
        (3, 2, Some("up")),
        (1, 2, Some("down")),
        (1, 3, None),
        (2, 2, None),
    ];
    for (source, destination, step) in cases {
        assert_eq!(
            RemoteOperation::workspace_step(source, destination, 5),
            step,
            "{source} -> {destination}"
        );
    }
}
