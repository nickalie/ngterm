//! The commands a remote window refuses (upstream's `ControlServer+RemoteActions.swift` `remoteRoute` and
//! `RemoteOperation.structuralCommands`): a command that would make, change or end a row there acts on this machine's
//! copy of the server's catalog, which the next catalog frame overwrites. Upstream sends the structural ones to the
//! server; until this build does, every one of them is refused with upstream's local-only refusal.

use ngterm_proto::command::{LiveCmd, RecentCmd, SessionCmd, WorkspaceCmd};
use ngterm_proto::{Command, Request};
use uuid::Uuid;

use crate::Core;
use crate::error::ControlError;
use crate::resolve::Resolver;

/// The commands that make, change or end a row, a split or a workspace (upstream's `structuralCommands` and
/// `localOnlyCommands`).
const ROW_COMMANDS: [Command; 21] = [
    Command::Session(SessionCmd::New),
    Command::Session(SessionCmd::Duplicate),
    Command::Session(SessionCmd::CompanionNew),
    Command::Session(SessionCmd::CompanionPage),
    Command::Session(SessionCmd::Page),
    Command::Session(SessionCmd::Split),
    Command::Session(SessionCmd::SplitClose),
    Command::Session(SessionCmd::Swap),
    Command::Session(SessionCmd::Rename),
    Command::Session(SessionCmd::Close),
    Command::Session(SessionCmd::Move),
    Command::Session(SessionCmd::Scratch),
    Command::Session(SessionCmd::CompanionDuplicate),
    Command::Session(SessionCmd::CompanionConvert),
    Command::Workspace(WorkspaceCmd::New),
    Command::Workspace(WorkspaceCmd::Rename),
    Command::Workspace(WorkspaceCmd::Delete),
    Command::Workspace(WorkspaceCmd::Move),
    Command::Live(LiveCmd::Attach),
    Command::Live(LiveCmd::AttachGroup),
    Command::Recent(RecentCmd::Open),
];

/// The refusal of a row command whose window is a remote window; `None` for any other command or window, and for a
/// target that does not resolve, whose own dispatch names the error.
pub(crate) fn refusal(core: &Core, request: &Request) -> Option<ControlError> {
    let cmd = request.cmd;
    if !ROW_COMMANDS.contains(&cmd) {
        return None;
    }
    let window = routed_window(core, request)?;
    core.library()
        .is_remote_window(window)
        .then(|| ControlError::rejected(format!("{cmd} is not available in a remote window")))
}

/// The window the local dispatch would act on (upstream's `routedStore`): the placement window for a command that
/// makes a row, else the target's owner.
fn routed_window(core: &Core, request: &Request) -> Option<Uuid> {
    let resolver = Resolver::new(core.library());
    let args = request.args.as_ref();
    let window = args.and_then(|args| args.window.as_deref());
    match request.cmd {
        Command::Session(SessionCmd::New)
        | Command::Workspace(WorkspaceCmd::New)
        | Command::Live(_)
        | Command::Recent(_) => resolver.placement_window(window).ok().flatten(),
        Command::Workspace(_) => resolver
            .workspace(request.target.as_deref(), window)
            .ok()
            .map(|target| target.window),
        _ => {
            let first = args
                .and_then(|args| args.targets.as_ref())
                .and_then(|targets| targets.first());
            let target = first.map(String::as_str).or(request.target.as_deref());
            resolver.session(target, window).ok().map(|target| target.window)
        }
    }
}
