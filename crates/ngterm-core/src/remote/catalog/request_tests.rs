//! Upstream's `RemoteOperationRequestTests`.

use ngterm_proto::command::{SessionCmd, WorkspaceCmd};
use ngterm_proto::{Args, Command, Request};
use uuid::Uuid;

use super::*;
use crate::model::{AppStore, NewSession};
use crate::pane_env::uuid_string;

/// A command, the field it is asked with and how the field is set.
type FieldCase = (Command, &'static str, fn(&mut Args));

struct Rows {
    store: AppStore,
    workspace: Uuid,
    first: Uuid,
    second: Uuid,
}

impl Rows {
    fn new() -> Self {
        let mut store = AppStore::new();
        let workspace = store.add_workspace("w");
        let first = store.add_session(workspace, NewSession::at_cwd("/a")).unwrap();
        let second = store.add_session(workspace, NewSession::at_cwd("/b")).unwrap();
        store.select_session(Some(first));
        Self {
            store,
            workspace,
            first,
            second,
        }
    }

    fn parse(&self, cmd: Command, target: Option<Uuid>, configure: impl FnOnce(&mut Args)) -> Parsed {
        let mut args = Args::default();
        configure(&mut args);
        let request = Request {
            cmd,
            target: target.map(uuid_string),
            args: Some(args),
        };
        let catalog: Vec<Uuid> = self.store.sessions().map(crate::model::Session::id).collect();
        parse(&request, &self.store, &catalog)
    }
}

fn session(cmd: SessionCmd) -> Command {
    Command::Session(cmd)
}

fn shell(name: Option<&str>, cwd: Option<&str>) -> RemoteShellOptions {
    RemoteShellOptions {
        name: name.map(str::to_owned),
        cwd: cwd.map(str::to_owned),
        command: None,
    }
}

#[test]
fn a_new_session_lands_in_the_named_workspace_with_its_shell_options() {
    let rows = Rows::new();
    let prefix = uuid_string(rows.workspace)[..8].to_owned();
    let parsed = rows.parse(session(SessionCmd::New), None, |a| {
        a.workspace = Some(prefix);
        a.name = Some("-x".to_owned());
        a.cwd = Some("/srv".to_owned());
    });
    let operation = RemoteOperation::NewSession {
        workspace: rows.workspace,
        anchor: None,
        shell: shell(Some("-x"), Some("/srv")),
    };
    assert_eq!(parsed, Ok((operation, true)));
}

#[test]
fn a_new_session_beside_an_anchor_takes_the_anchors_workspace() {
    let rows = Rows::new();
    let parsed = rows.parse(session(SessionCmd::New), None, |a| {
        a.before = Some(uuid_string(rows.second));
        a.no_select = Some(true);
    });
    let operation = RemoteOperation::NewSession {
        workspace: rows.workspace,
        anchor: Some(RemoteSessionPlacement::Before(rows.second)),
        shell: RemoteShellOptions::default(),
    };
    assert_eq!(parsed, Ok((operation, false)));
    let both = rows.parse(session(SessionCmd::New), None, |a| {
        a.after = Some(uuid_string(rows.first));
        a.workspace = Some(uuid_string(rows.workspace));
    });
    assert_eq!(
        both,
        Err("session.new takes --after/--before or a workspace, not both".to_owned())
    );
    let two = rows.parse(session(SessionCmd::New), None, |a| {
        a.after = Some(uuid_string(rows.first));
        a.before = Some(uuid_string(rows.second));
    });
    assert_eq!(two, Err("use either --after or --before, not both".to_owned()));
}

#[test]
fn a_duplicate_is_a_fresh_shell_after_the_row_in_its_directory() {
    let rows = Rows::new();
    let parsed = rows.parse(session(SessionCmd::Duplicate), Some(rows.first), |_| {});
    let operation = RemoteOperation::NewSession {
        workspace: rows.workspace,
        anchor: Some(RemoteSessionPlacement::After(rows.first)),
        shell: shell(None, Some("/a")),
    };
    assert_eq!(parsed, Ok((operation, true)));
}

#[test]
fn a_scratch_resolves_its_mode_against_the_rows_scratch() {
    let mut rows = Rows::new();
    for (mode, shown) in [(None, true), (Some("on"), true), (Some("off"), false)] {
        let parsed = rows.parse(session(SessionCmd::Scratch), Some(rows.first), |a| {
            a.mode = mode.map(str::to_owned);
        });
        let operation = RemoteOperation::SetScratch {
            session: rows.first,
            shown,
            command: None,
        };
        assert_eq!(parsed, Ok((operation, false)), "{mode:?}");
    }
    rows.store.session_mut(rows.first).unwrap().scratch_active = true;
    let toggled = rows.parse(session(SessionCmd::Scratch), Some(rows.first), |_| {});
    let off = RemoteOperation::SetScratch {
        session: rows.first,
        shown: false,
        command: None,
    };
    assert_eq!(toggled, Ok((off, false)));
    let followed = rows.parse(session(SessionCmd::Scratch), Some(rows.first), |a| {
        a.mode = Some("on".to_owned());
        a.command = Some("htop".to_owned());
        a.follow = Some(true);
    });
    let on = RemoteOperation::SetScratch {
        session: rows.first,
        shown: true,
        command: Some("htop".to_owned()),
    };
    assert_eq!(followed, Ok((on, true)));
}

#[test]
fn a_scratch_refuses_an_expected_scratch_and_a_bad_mode() {
    let rows = Rows::new();
    let expected = rows.parse(session(SessionCmd::Scratch), None, |a| {
        a.scratch = Some("none".to_owned());
    });
    let sideways = rows.parse(session(SessionCmd::Scratch), None, |a| {
        a.mode = Some("sideways".to_owned());
    });
    assert_eq!(
        expected,
        Err("operation receipts are served only by ngterm-headless".to_owned())
    );
    assert_eq!(sideways, Err("invalid scratch mode: sideways".to_owned()));
}

#[test]
fn a_companion_lands_under_the_active_session() {
    let rows = Rows::new();
    let parsed = rows.parse(session(SessionCmd::CompanionNew), None, |_| {});
    let operation = RemoteOperation::NewCompanion {
        parent: rows.first,
        shell: RemoteShellOptions::default(),
    };
    assert_eq!(parsed, Ok((operation, true)));
}

#[test]
fn a_companion_with_a_placement_is_refused() {
    let rows = Rows::new();
    let parsed = rows.parse(session(SessionCmd::CompanionNew), None, |a| {
        a.after = Some(uuid_string(rows.second));
    });
    assert_eq!(
        parsed,
        Err("a companion takes no workspace or placement: it lands under its parent".to_owned())
    );
}

#[test]
fn a_split_resolves_its_mode_against_the_row() {
    let mut rows = Rows::new();
    for (mode, shown) in [(None, true), (Some("on"), true), (Some("off"), false)] {
        let parsed = rows.parse(session(SessionCmd::Split), Some(rows.first), |a| {
            a.mode = mode.map(str::to_owned);
            a.axis = Some("horizontal".to_owned());
        });
        let operation = RemoteOperation::SetSplit {
            session: rows.first,
            shown,
            axis: Some("horizontal".to_owned()),
        };
        assert_eq!(parsed, Ok((operation, false)), "{mode:?}");
    }
    let row = rows.store.session_mut(rows.first).unwrap();
    row.split.exists = true;
    row.split.shown = true;
    let toggled = rows.parse(session(SessionCmd::Split), Some(rows.first), |_| {});
    let hide = RemoteOperation::SetSplit {
        session: rows.first,
        shown: false,
        axis: None,
    };
    assert_eq!(toggled, Ok((hide, false)));
    let axis = rows.parse(session(SessionCmd::Split), None, |a| {
        a.axis = Some("diagonal".to_owned());
    });
    let mode = rows.parse(session(SessionCmd::Split), None, |a| a.mode = Some("maybe".to_owned()));
    assert_eq!(
        axis,
        Err("invalid split axis: diagonal (vertical|horizontal)".to_owned())
    );
    assert_eq!(mode, Err("session.split mode must be on, off or toggle".to_owned()));
}

#[test]
fn every_session_verb_resolves_its_target() {
    let rows = Rows::new();
    let target = Some(rows.first);
    let none = |_: &mut Args| {};
    assert_eq!(
        rows.parse(session(SessionCmd::SplitClose), target, none),
        Ok((RemoteOperation::CloseSplit(rows.first), false))
    );
    assert_eq!(
        rows.parse(session(SessionCmd::Swap), target, none),
        Ok((RemoteOperation::Swap(rows.first), false))
    );
    let rename = RemoteOperation::RenameSession {
        session: rows.first,
        name: "build".to_owned(),
    };
    assert_eq!(
        rows.parse(session(SessionCmd::Rename), target, |a| a.name =
            Some("build".to_owned())),
        Ok((rename, false))
    );
    assert_eq!(
        rows.parse(session(SessionCmd::Rename), target, none),
        Err("session.rename requires a name".to_owned())
    );
    assert_eq!(
        rows.parse(session(SessionCmd::Close), target, none),
        Ok((RemoteOperation::CloseSession(rows.first), false))
    );
    let batch_of_one = rows.parse(session(SessionCmd::Close), None, |a| {
        a.targets = Some(vec![uuid_string(rows.first)]);
    });
    assert_eq!(batch_of_one, Ok((RemoteOperation::CloseSession(rows.first), false)));
}

#[test]
fn a_close_reaches_a_session_the_catalog_lists_beyond_the_rows() {
    let rows = Rows::new();
    let hidden = Uuid::new_v4();
    let request = Request {
        cmd: session(SessionCmd::Close),
        target: Some(uuid_string(hidden)[..8].to_owned()),
        args: None,
    };
    let parsed = parse(&request, &rows.store, &[rows.first, hidden]);
    assert_eq!(parsed, Ok((RemoteOperation::CloseSession(hidden), false)));
}

#[test]
fn a_move_takes_exactly_one_placement() {
    let rows = Rows::new();
    let target = Some(rows.first);
    let moved = |to| {
        Ok((
            RemoteOperation::MoveSession {
                session: rows.first,
                to,
            },
            false,
        ))
    };
    assert_eq!(
        rows.parse(session(SessionCmd::Move), target, |a| a.to = Some("down".to_owned())),
        moved(RemoteSessionPlacement::Step("down".to_owned()))
    );
    assert_eq!(
        rows.parse(session(SessionCmd::Move), target, |a| a.after =
            Some(uuid_string(rows.second))),
        moved(RemoteSessionPlacement::After(rows.second))
    );
    assert_eq!(
        rows.parse(session(SessionCmd::Move), target, |a| a.before =
            Some(uuid_string(rows.second))),
        moved(RemoteSessionPlacement::Before(rows.second))
    );
    assert_eq!(
        rows.parse(session(SessionCmd::Move), target, |a| a.workspace =
            Some(uuid_string(rows.workspace))),
        moved(RemoteSessionPlacement::Workspace(rows.workspace))
    );
    let two = rows.parse(session(SessionCmd::Move), target, |a| {
        a.to = Some("down".to_owned());
        a.workspace = Some(uuid_string(rows.workspace));
    });
    assert_eq!(
        two,
        Err("session.move takes exactly one of --to, a workspace, or --after/--before".to_owned())
    );
    let sideways = rows.parse(session(SessionCmd::Move), target, |a| {
        a.to = Some("sideways".to_owned());
    });
    assert_eq!(sideways, Err("session.move --to must be up|down|top|bottom".to_owned()));
    let both = rows.parse(session(SessionCmd::Move), target, |a| {
        a.after = Some(uuid_string(rows.second));
        a.before = Some(uuid_string(rows.second));
    });
    assert_eq!(both, Err("use either --after or --before, not both".to_owned()));
}

#[test]
fn every_workspace_verb_resolves_its_target() {
    let rows = Rows::new();
    let target = Some(rows.workspace);
    let workspace = |cmd| Command::Workspace(cmd);
    assert_eq!(
        rows.parse(workspace(WorkspaceCmd::New), None, |a| a.name = Some("dev".to_owned())),
        Ok((
            RemoteOperation::NewWorkspace {
                name: Some("dev".to_owned())
            },
            false
        ))
    );
    let rename = RemoteOperation::RenameWorkspace {
        workspace: rows.workspace,
        name: "ops".to_owned(),
    };
    assert_eq!(
        rows.parse(workspace(WorkspaceCmd::Rename), target, |a| a.name =
            Some(" ops ".to_owned())),
        Ok((rename, false))
    );
    assert_eq!(
        rows.parse(workspace(WorkspaceCmd::Rename), target, |a| a.name =
            Some(" ".to_owned())),
        Err("workspace.rename requires a name".to_owned())
    );
    assert_eq!(
        rows.parse(workspace(WorkspaceCmd::Delete), target, |_| {}),
        Ok((RemoteOperation::DeleteWorkspace(rows.workspace), false))
    );
    let moved = RemoteOperation::MoveWorkspace {
        workspace: rows.workspace,
        to: "top".to_owned(),
    };
    assert_eq!(
        rows.parse(workspace(WorkspaceCmd::Move), target, |a| a.to = Some("top".to_owned())),
        Ok((moved, false))
    );
    assert_eq!(
        rows.parse(workspace(WorkspaceCmd::Move), target, |_| {}),
        Err("workspace.move --to must be up|down|top|bottom".to_owned())
    );
}

#[test]
fn a_field_the_server_cannot_be_asked_for_is_refused_by_name() {
    let rows = Rows::new();
    let cases: [FieldCase; 9] = [
        (session(SessionCmd::New), "--workspace-name", |a| {
            a.workspace_name = Some("w".to_owned());
        }),
        (session(SessionCmd::New), "--create-workspace", |a| {
            a.create_workspace = Some(true);
        }),
        (session(SessionCmd::New), "--wait", |a| a.wait = Some(true)),
        (session(SessionCmd::New), "--tint", |a| a.tint = Some("red".to_owned())),
        (session(SessionCmd::CompanionNew), "--wait", |a| a.wait = Some(true)),
        (session(SessionCmd::CompanionNew), "--tint", |a| {
            a.tint_on = Some(vec!["icon".to_owned()]);
        }),
        (session(SessionCmd::Close), "--undoable", |a| a.undoable = Some(true)),
        (Command::Workspace(WorkspaceCmd::New), "--collapsed", |a| {
            a.collapsed = Some(true);
        }),
        (Command::Workspace(WorkspaceCmd::New), "--tint", |a| {
            a.tint = Some("red".to_owned());
        }),
    ];
    for (cmd, field, configure) in cases {
        let parsed = rows.parse(cmd, Some(rows.first), configure);
        assert_eq!(
            parsed,
            Err(format!("{field} is not available in a remote window")),
            "{cmd}"
        );
    }
}

#[test]
fn a_request_carrying_its_own_operation_is_refused_before_anything_is_sent() {
    let rows = Rows::new();
    let cases: [fn(&mut Args); 3] = [
        |a| a.op_id = Some("op-1".to_owned()),
        |a| a.epoch = Some("e1".to_owned()),
        |a| a.panes = Some(vec![uuid_string(Uuid::new_v4())]),
    ];
    for configure in cases {
        let parsed = rows.parse(session(SessionCmd::Rename), Some(rows.first), |a| {
            a.name = Some("x".to_owned());
            configure(a);
        });
        assert_eq!(
            parsed,
            Err("operation receipts are served only by ngterm-headless".to_owned())
        );
    }
}

#[test]
fn a_batch_is_refused() {
    let rows = Rows::new();
    let targets = Some(vec![uuid_string(rows.first), uuid_string(rows.second)]);
    let close = rows.parse(session(SessionCmd::Close), None, |a| a.targets.clone_from(&targets));
    let moved = rows.parse(session(SessionCmd::Move), None, |a| {
        a.targets.clone_from(&targets);
        a.to = Some("up".to_owned());
    });
    assert_eq!(close, Err("a remote window closes one session at a time".to_owned()));
    assert_eq!(moved, Err("a remote window moves one session at a time".to_owned()));
}

#[test]
fn an_unknown_target_is_refused_with_the_resolvers_error() {
    let rows = Rows::new();
    let request = Request {
        cmd: session(SessionCmd::Rename),
        target: Some("zzzz".to_owned()),
        args: Some(Args {
            name: Some("x".to_owned()),
            ..Args::default()
        }),
    };
    assert_eq!(
        parse(&request, &rows.store, &[]),
        Err("no such session: zzzz".to_owned())
    );
}

#[test]
fn a_command_outside_the_structural_set_is_not_sent() {
    let rows = Rows::new();
    let parsed = rows.parse(session(SessionCmd::Focus), Some(rows.first), |_| {});
    assert_eq!(
        parsed,
        Err("session.focus is not sent to a remote window's server".to_owned())
    );
}

#[test]
fn a_new_session_after_an_anchor_and_commands_outside_the_set_are_read() {
    let rows = Rows::new();
    let after = rows.parse(session(SessionCmd::New), None, |a| {
        a.after = Some(uuid_string(rows.first));
    });
    let operation = RemoteOperation::NewSession {
        workspace: rows.workspace,
        anchor: Some(RemoteSessionPlacement::After(rows.first)),
        shell: RemoteShellOptions::default(),
    };
    assert_eq!(after, Ok((operation, true)));
    let list = rows.parse(Command::Workspace(WorkspaceCmd::Select), None, |_| {});
    assert_eq!(
        list,
        Err("workspace.select is not sent to a remote window's server".to_owned())
    );
    let tree = rows.parse(Command::Tree, None, |_| {});
    assert_eq!(tree, Err("tree is not sent to a remote window's server".to_owned()));
}
