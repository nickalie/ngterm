//! A control request read as a remote window's operation (upstream's `RemoteOperation+Request.swift`): its targets
//! resolve in the remote window's rows, and a field the server cannot be asked for from here is refused by name rather
//! than dropped.

use ngterm_proto::command::{SessionCmd, WorkspaceCmd};
use ngterm_proto::{Args, Command, Request};
use uuid::Uuid;

use super::operation::{RemoteOperation, RemoteSessionPlacement, RemoteShellOptions};
use crate::commands::ToggleMode;
use crate::dispatch::OPERATION_RECEIPTS_ERROR;
use crate::error::Noun;
use crate::model::{AppStore, ReorderDirection, SplitAxis};
use crate::resolve::{ACTIVE, resolve_id, session_ids, trimmed, workspace_ids};

/// The commands a remote window sends to its server instead of running on this machine (upstream's
/// `structuralCommands`, its page rows left to the page commands).
pub(crate) const STRUCTURAL_COMMANDS: [Command; 14] = [
    Command::Session(SessionCmd::New),
    Command::Session(SessionCmd::Duplicate),
    Command::Session(SessionCmd::CompanionNew),
    Command::Session(SessionCmd::Split),
    Command::Session(SessionCmd::SplitClose),
    Command::Session(SessionCmd::Swap),
    Command::Session(SessionCmd::Rename),
    Command::Session(SessionCmd::Close),
    Command::Session(SessionCmd::Move),
    Command::Session(SessionCmd::Scratch),
    Command::Workspace(WorkspaceCmd::New),
    Command::Workspace(WorkspaceCmd::Rename),
    Command::Workspace(WorkspaceCmd::Delete),
    Command::Workspace(WorkspaceCmd::Move),
];

/// A request read as an operation, with whether its created row is selected once the window shows it; the refusal
/// to answer with otherwise.
pub(crate) type Parsed = Result<(RemoteOperation, bool), String>;

/// Reads `request` against the remote window `store`; `catalog` lists the server's sessions, which a close reaches.
pub(crate) fn parse(request: &Request, store: &AppStore, catalog: &[Uuid]) -> Parsed {
    let default = Args::default();
    let args = request.args.as_ref().unwrap_or(&default);
    // the caller's own id and epoch would be replaced by this window's, running what its epoch might refuse
    if args.op_id.is_some() || args.epoch.is_some() || args.panes.is_some() || args.scratch.is_some() {
        return Err(OPERATION_RECEIPTS_ERROR.to_owned());
    }
    if let Some(field) = unsupported_field(request.cmd, args) {
        return Err(format!("{field} is not available in a remote window"));
    }
    let target = request.target.as_deref();
    match request.cmd {
        Command::Session(cmd) => parse_session(cmd, target, args, store, catalog),
        Command::Workspace(cmd) => parse_workspace(cmd, target, args, store),
        cmd => Err(format!("{cmd} is not sent to a remote window's server")),
    }
}

fn parse_session(cmd: SessionCmd, target: Option<&str>, args: &Args, store: &AppStore, catalog: &[Uuid]) -> Parsed {
    let select = args.no_select != Some(true);
    let shell = RemoteShellOptions {
        name: args.name.clone(),
        cwd: args.cwd.clone(),
        command: args.command.clone(),
    };
    match cmd {
        SessionCmd::New => new_session(args, store, shell, select),
        SessionCmd::Duplicate => {
            // the server makes a fresh shell after the row, in the directory this window shows for it
            let id = session(target, store)?;
            let workspace = workspace_of(store, id)?;
            let shell = RemoteShellOptions {
                cwd: store.session(id).map(|row| row.focused_cwd().to_owned()),
                ..RemoteShellOptions::default()
            };
            let anchor = Some(RemoteSessionPlacement::After(id));
            Ok((
                RemoteOperation::NewSession {
                    workspace,
                    anchor,
                    shell,
                },
                true,
            ))
        }
        SessionCmd::CompanionNew => {
            if args.workspace.is_some() || args.after.is_some() || args.before.is_some() {
                return Err("a companion takes no workspace or placement: it lands under its parent".to_owned());
            }
            let parent = session(target, store)?;
            Ok((RemoteOperation::NewCompanion { parent, shell }, select))
        }
        SessionCmd::Split => split(target, args, store),
        SessionCmd::Scratch => scratch(target, args, store),
        SessionCmd::SplitClose => Ok((RemoteOperation::CloseSplit(session(target, store)?), false)),
        SessionCmd::Swap => Ok((RemoteOperation::Swap(session(target, store)?), false)),
        SessionCmd::Rename => {
            let name = args.name.clone().ok_or("session.rename requires a name")?;
            let session = session(target, store)?;
            Ok((RemoteOperation::RenameSession { session, name }, false))
        }
        SessionCmd::Close => {
            let target = single_target(target, args).ok_or("a remote window closes one session at a time")?;
            // a close ends the session on the server, so it reaches every session the catalog lists
            let id =
                resolve_id(Noun::Session, target, catalog, store.selected_session_id()).map_err(|e| e.to_string())?;
            Ok((RemoteOperation::CloseSession(id), false))
        }
        SessionCmd::Move => {
            let target = single_target(target, args).ok_or("a remote window moves one session at a time")?;
            let id = session(Some(target), store)?;
            Ok((move_session(id, args, store)?, false))
        }
        cmd => Err(format!(
            "{} is not sent to a remote window's server",
            Command::Session(cmd)
        )),
    }
}

fn new_session(args: &Args, store: &AppStore, shell: RemoteShellOptions, select: bool) -> Parsed {
    if args.after.is_some() && args.before.is_some() {
        return Err("use either --after or --before, not both".to_owned());
    }
    let Some(anchor) = args.after.as_deref().or(args.before.as_deref()) else {
        let workspace = workspace(args.workspace.as_deref(), store)?;
        return Ok((
            RemoteOperation::NewSession {
                workspace,
                anchor: None,
                shell,
            },
            select,
        ));
    };
    if args.workspace.is_some() {
        return Err("session.new takes --after/--before or a workspace, not both".to_owned());
    }
    let id = session(Some(anchor), store)?;
    let workspace = workspace_of(store, id)?;
    let anchor = Some(if args.after.is_some() {
        RemoteSessionPlacement::After(id)
    } else {
        RemoteSessionPlacement::Before(id)
    });
    Ok((
        RemoteOperation::NewSession {
            workspace,
            anchor,
            shell,
        },
        select,
    ))
}

fn split(target: Option<&str>, args: &Args, store: &AppStore) -> Parsed {
    if let Some(axis) = args.axis.as_deref()
        && SplitAxis::from_wire(axis).is_none()
    {
        return Err(format!("invalid split axis: {axis} (vertical|horizontal)"));
    }
    let mode = ToggleMode::parse(args.mode.as_deref()).ok_or("session.split mode must be on, off or toggle")?;
    let id = session(target, store)?;
    let shown = mode.desired_value(store.session(id).is_some_and(|row| row.split.shown));
    let operation = RemoteOperation::SetSplit {
        session: id,
        shown,
        axis: args.axis.clone(),
    };
    Ok((operation, false))
}

fn scratch(target: Option<&str>, args: &Args, store: &AppStore) -> Parsed {
    let mode = ToggleMode::parse(args.mode.as_deref())
        .ok_or_else(|| format!("invalid scratch mode: {}", args.mode.as_deref().unwrap_or("toggle")))?;
    let id = session(target, store)?;
    let shown = mode.desired_value(store.session(id).is_some_and(|row| row.scratch_active));
    let command = args.command.clone().filter(|command| shown && !command.is_empty());
    let operation = RemoteOperation::SetScratch {
        session: id,
        shown,
        command,
    };
    Ok((operation, shown && args.follow == Some(true)))
}

fn move_session(id: Uuid, args: &Args, store: &AppStore) -> Result<RemoteOperation, String> {
    if args.after.is_some() && args.before.is_some() {
        return Err("use either --after or --before, not both".to_owned());
    }
    let anchored = args.after.is_some() || args.before.is_some();
    let placements = [anchored, args.to.is_some(), args.workspace.is_some()];
    if placements.iter().filter(|placed| **placed).count() != 1 {
        return Err("session.move takes exactly one of --to, a workspace, or --after/--before".to_owned());
    }
    let to = if let Some(anchor) = args.after.as_deref().or(args.before.as_deref()) {
        let anchor = session(Some(anchor), store)?;
        if args.after.is_some() {
            RemoteSessionPlacement::After(anchor)
        } else {
            RemoteSessionPlacement::Before(anchor)
        }
    } else if let Some(to) = args.to.as_deref() {
        ReorderDirection::from_wire(to).ok_or("session.move --to must be up|down|top|bottom")?;
        RemoteSessionPlacement::Step(to.to_owned())
    } else {
        RemoteSessionPlacement::Workspace(workspace(args.workspace.as_deref(), store)?)
    };
    Ok(RemoteOperation::MoveSession { session: id, to })
}

fn parse_workspace(cmd: WorkspaceCmd, target: Option<&str>, args: &Args, store: &AppStore) -> Parsed {
    let operation = match cmd {
        WorkspaceCmd::New => RemoteOperation::NewWorkspace {
            name: args.name.clone(),
        },
        WorkspaceCmd::Rename => {
            let name = trimmed(args.name.as_deref()).ok_or("workspace.rename requires a name")?;
            RemoteOperation::RenameWorkspace {
                workspace: workspace(target, store)?,
                name: name.to_owned(),
            }
        }
        WorkspaceCmd::Delete => RemoteOperation::DeleteWorkspace(workspace(target, store)?),
        WorkspaceCmd::Move => {
            let to = args
                .to
                .as_deref()
                .filter(|to| ReorderDirection::from_wire(to).is_some())
                .ok_or("workspace.move --to must be up|down|top|bottom")?;
            RemoteOperation::MoveWorkspace {
                workspace: workspace(target, store)?,
                to: to.to_owned(),
            }
        }
        cmd => {
            return Err(format!(
                "{} is not sent to a remote window's server",
                Command::Workspace(cmd)
            ));
        }
    };
    Ok((operation, false))
}

fn unsupported_field(cmd: Command, args: &Args) -> Option<&'static str> {
    let tinted = args.tint.is_some() || args.tint_on.is_some();
    let waits = args.wait == Some(true);
    let fields: &[(bool, &'static str)] = match cmd {
        Command::Session(SessionCmd::New) => &[
            (args.workspace_name.is_some(), "--workspace-name"),
            (args.create_workspace == Some(true), "--create-workspace"),
            (waits, "--wait"),
            (tinted, "--tint"),
        ],
        Command::Session(SessionCmd::CompanionNew) => &[(waits, "--wait"), (tinted, "--tint")],
        Command::Session(SessionCmd::Close) => &[(args.undoable == Some(true), "--undoable")],
        Command::Workspace(WorkspaceCmd::New) => &[(args.collapsed == Some(true), "--collapsed"), (tinted, "--tint")],
        _ => &[],
    };
    fields.iter().find(|(set, _)| *set).map(|(_, field)| *field)
}

/// The request's one target, from `target` or a one-element `targets`; `None` for a batch.
fn single_target<'a>(target: Option<&'a str>, args: &'a Args) -> Option<&'a str> {
    match args.targets.as_deref() {
        None => Some(target.unwrap_or(ACTIVE)),
        Some([only]) => Some(only.as_str()),
        Some(_) => None,
    }
}

fn session(target: Option<&str>, store: &AppStore) -> Result<Uuid, String> {
    let target = target.unwrap_or(ACTIVE);
    resolve_id(Noun::Session, target, &session_ids(store), store.selected_session_id()).map_err(|e| e.to_string())
}

fn workspace(target: Option<&str>, store: &AppStore) -> Result<Uuid, String> {
    let target = target.unwrap_or(ACTIVE);
    resolve_id(
        Noun::Workspace,
        target,
        &workspace_ids(store),
        store.current_workspace_id(),
    )
    .map_err(|e| e.to_string())
}

fn workspace_of(store: &AppStore, session: Uuid) -> Result<Uuid, String> {
    store
        .session_location(session)
        .map(|location| location.workspace)
        .ok_or_else(|| "no such session".to_owned())
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
