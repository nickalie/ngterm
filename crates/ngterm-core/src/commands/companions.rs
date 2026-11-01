//! `session.companion.new`, `session.collapse` and `session.expand`: a companion session under a parent, and folding
//! a parent's companions. Validation and its messages follow upstream's `ControlDispatcher+Companions.swift`; the
//! effects follow `createCompanion`, `createLocalCompanion` and `setSessionExpansion` in
//! `agterm/Control/ControlServer+Companions.swift`.
//!
//! Upstream's `ControlActions` default that refuses companions on an older host has no target here: one dispatcher
//! serves the app and `ngterm-headless`. A remote row's companion is its origin's to create ([`begin_remote`]); the
//! store's remote-parent refusal stays as upstream's local-path one. The page forms are [`super::companion_page`]'s,
//! the duplicate form [`super::companion_duplicate`]'s.

use std::time::Duration;

use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Args, Command, Request};

use super::{Outcome, args_of, session_store, with_id};
use crate::Core;
use crate::dispatch::Dispatched;
use crate::error::ControlError;
use crate::model::{CompanionOptions, CompanionRefusal};
use crate::pane_env::uuid_string;
use crate::remote::companions::RemoteCompanionRequest;
use crate::resolve::Resolver;

/// The refusal of a session move that would take a companion out of its group.
pub(crate) const COMPANION_MOVE_ERROR: &str = "a companion moves only among its siblings";

/// Answers the companion commands.
pub(crate) fn dispatch(core: &mut Core, cmd: SessionCmd, request: &Request) -> Outcome {
    let args = args_of(request);
    let target = request.target.as_deref();
    match cmd {
        SessionCmd::CompanionNew => new(core, target, args),
        SessionCmd::Collapse => set_expanded(core, target, args, false),
        SessionCmd::Expand => set_expanded(core, target, args, true),
        SessionCmd::CompanionPage => super::companion_page::page(core, target, args),
        SessionCmd::CompanionPageAttach => super::companion_page::attach(core, target, args.window.as_deref()),
        SessionCmd::CompanionDuplicate => super::companion_duplicate::duplicate(core, request),
        other => Err(ControlError::NotSupported(Command::Session(other))),
    }
}

/// Starts a companion command whose target is a row attached from another machine (upstream's
/// `createRemoteCompanion` branches of `createCompanion`, `createPageCompanion` and `duplicateAsCompanion`): the
/// command's own checks first, then the origin creates it ([`crate::remote::companions`]). `None` for any other target,
/// which the command answers at once.
pub(crate) fn begin_remote(core: &mut Core, request: &Request) -> Option<Dispatched> {
    let Command::Session(cmd @ (SessionCmd::CompanionNew | SessionCmd::CompanionPage | SessionCmd::CompanionDuplicate)) =
        request.cmd
    else {
        return None;
    };
    let args = args_of(request);
    let resolved = Resolver::new(core.library())
        .session(request.target.as_deref(), args.window.as_deref())
        .ok()?;
    core.library()
        .store(resolved.window)?
        .session(resolved.session)?
        .remote_host()?;
    let remote = match cmd {
        SessionCmd::CompanionNew => validated_options(args).map(|options| {
            let select = options.select;
            (RemoteCompanionRequest::Terminal(options), select)
        }),
        SessionCmd::CompanionPage => super::companion_page::remote_page(core, args),
        _ => Ok((RemoteCompanionRequest::Duplicate, args.no_select != Some(true))),
    };
    let started = remote.and_then(|(remote, select)| {
        core.start_remote_companion(remote, resolved.session, select)
            .map_err(ControlError::rejected)
    });
    Some(match started {
        Ok(flow) => crate::remote::companions::drive(flow, Duration::ZERO),
        Err(error) => Dispatched::done(Err(error)),
    })
}

/// What `session.companion.new` asks for, once its own checks passed.
fn validated_options(args: &Args) -> Result<CompanionOptions, ControlError> {
    if args.workspace.is_some()
        || args.workspace_name.is_some()
        || args.create_workspace == Some(true)
        || args.after.is_some()
        || args.before.is_some()
    {
        return Err(ControlError::rejected(
            "a companion takes no workspace or placement: it lands under its parent",
        ));
    }
    if args.wait == Some(true) && args.command.is_none() {
        return Err(ControlError::rejected("--wait requires --command"));
    }
    let tint = super::tint::creation_tint(args)?;
    Ok(CompanionOptions {
        cwd: args.cwd.clone(),
        command: args.command.clone(),
        name: args.name.clone(),
        wait: args.wait == Some(true),
        select: args.no_select != Some(true),
        tint,
    })
}

/// `session.companion.new`: a companion lands under its parent, so a raw client's workspace or placement field is
/// refused, not dropped; the target's parent stands in for a companion target.
fn new(core: &mut Core, target: Option<&str>, args: &Args) -> Outcome {
    let options = validated_options(args)?;
    let home = core.library().home().to_owned();
    let (store, id) = session_store(core, target, args.window.as_deref())?;
    let companion = store
        .add_companion(id, options, &home)
        .map_err(|refusal| match refusal {
            CompanionRefusal::RemoteParent => {
                ControlError::rejected("companions of a remote session are not supported yet")
            }
            CompanionRefusal::NoSuchSession | CompanionRefusal::PageNotOpened(_) => {
                ControlError::rejected("could not create companion")
            }
        })?;
    let mut result = with_id(companion);
    result.parent = store
        .session(companion)
        .and_then(crate::model::Session::parent_id)
        .map(uuid_string);
    Ok(result)
}

/// `session.collapse`/`session.expand`: folds or unfolds a parent's companions, the sidebar row following even on an
/// unchanged value; a session without any is refused.
fn set_expanded(core: &mut Core, target: Option<&str>, args: &Args, expanded: bool) -> Outcome {
    let (store, id) = session_store(core, target, args.window.as_deref())?;
    if !store.set_session_expanded_in_outline(id, expanded) {
        return Err(ControlError::rejected("session has no companions"));
    }
    Ok(with_id(id))
}
