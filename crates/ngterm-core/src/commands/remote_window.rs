//! The commands a remote window sends to its server or refuses (upstream's `ControlServer+RemoteActions.swift`
//! `remoteRoute`): a command that makes, changes or ends a row there goes to the server under an operation id, as the
//! next catalog frame would overwrite any change made to this machine's copy. A command that makes a shell or a row on
//! this machine is refused with upstream's local-only refusal.

use ngterm_proto::command::{LiveCmd, RecentCmd, SessionCmd, WorkspaceCmd};
use ngterm_proto::{Command, Request};
use uuid::Uuid;

use crate::Core;
use crate::dispatch::Dispatched;
use crate::error::ControlError;
use crate::remote::catalog::request::{self, STRUCTURAL_COMMANDS};
use crate::resolve::Resolver;

/// The commands that make shells or rows on this machine, which a remote window shows only from its server
/// (upstream's `localOnlyCommands`), with the page rows a remote window does not show yet.
const LOCAL_ONLY_COMMANDS: [Command; 7] = [
    Command::Session(SessionCmd::CompanionDuplicate),
    Command::Session(SessionCmd::CompanionConvert),
    Command::Session(SessionCmd::CompanionPage),
    Command::Session(SessionCmd::Page),
    Command::Live(LiveCmd::Attach),
    Command::Live(LiveCmd::AttachGroup),
    Command::Recent(RecentCmd::Open),
];

/// Where a request on a remote window's rows goes.
pub(crate) enum Route {
    /// To the window's server.
    Send {
        window: Uuid,
        operation: crate::remote::catalog::RemoteOperation,
        select: bool,
    },
    Refused(ControlError),
}

/// Where `request` goes when it would act on a remote window's rows; `None` for any other command or window, and for
/// a target that does not resolve, whose own dispatch names the error.
pub(crate) fn route(core: &Core, request: &Request) -> Option<Route> {
    let cmd = request.cmd;
    let structural = STRUCTURAL_COMMANDS.contains(&cmd);
    if !structural && !LOCAL_ONLY_COMMANDS.contains(&cmd) {
        return None;
    }
    let window = routed_window(core, request)?;
    if !core.library().is_remote_window(window) {
        return None;
    }
    if !structural {
        return Some(Route::Refused(ControlError::rejected(format!(
            "{cmd} is not available in a remote window"
        ))));
    }
    let store = core.library().store(window)?;
    let catalog: Vec<Uuid> = core
        .remote
        .viewer
        .catalogs
        .get(&window)
        .and_then(|run| run.stream.catalog())
        .map(|catalog| {
            catalog
                .workspaces
                .iter()
                .flat_map(|workspace| {
                    workspace
                        .sessions
                        .iter()
                        .map(crate::catalog::frames::CatalogSession::id)
                })
                .collect()
        })
        .unwrap_or_default();
    Some(match request::parse(request, store, &catalog) {
        Ok((operation, select)) => Route::Send {
            window,
            operation,
            select,
        },
        Err(error) => Route::Refused(ControlError::rejected(error)),
    })
}

/// A control request's route: the server's reply once it comes, or the refusal.
pub(crate) fn begin(core: &mut Core, request: &Request) -> Option<Dispatched> {
    Some(match route(core, request)? {
        Route::Send {
            window,
            operation,
            select,
        } => core.begin_remote_operation(window, operation, select),
        Route::Refused(error) => Dispatched::done(Err(error)),
    })
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
