//! `session.detach|reattach` and `workspace.detach|reattach` (upstream's `ControlDispatcher+Detach.swift` and
//! `ControlServer+Detach.swift`): a remote window's rows hidden here and left running on the server, and shown again.
//! A local window's row refuses them.

use ngterm_proto::{ControlResult, Request};
use uuid::Uuid;

use super::args_of;
use crate::Core;
use crate::error::{ControlError, Noun};
use crate::pane_env::uuid_string;
use crate::remote::catalog::DetachedKind;
use crate::remote::catalog::actions::remote_window_only;
use crate::resolve::{ACTIVE, Resolver, TargetResolution, resolution_error, resolve, trimmed};

/// `session.detach`: the targets (default `active`) must be rows of one remote window.
pub(crate) fn detach_sessions(core: &mut Core, request: &Request) -> Result<ControlResult, ControlError> {
    let args = args_of(request);
    let targets = args
        .targets
        .clone()
        .unwrap_or_else(|| vec![request.target.clone().unwrap_or_else(|| ACTIVE.to_owned())]);
    if targets.is_empty() {
        return Err(ControlError::rejected("session.detach requires at least one --target"));
    }
    let resolver = Resolver::new(core.library());
    let mut owner = None;
    let mut ids: Vec<Uuid> = Vec::new();
    for target in &targets {
        let found = resolver.session(Some(target), args.window.as_deref())?;
        if owner.is_some_and(|owner| owner != found.window) {
            return Err(ControlError::rejected("session.detach takes sessions of one window"));
        }
        owner = Some(found.window);
        if !ids.contains(&found.session) {
            ids.push(found.session);
        }
    }
    let window = owner
        .filter(|window| core.remote.viewer.catalogs.contains_key(window))
        .ok_or_else(|| ControlError::rejected(remote_window_only("session.detach")))?;
    core.detach_sessions(window, &ids);
    Ok(ControlResult::affected(i64::try_from(ids.len()).unwrap_or(i64::MAX)))
}

/// `session.reattach`: a detached session's id or unique prefix.
pub(crate) fn reattach_session(core: &mut Core, request: &Request) -> Result<ControlResult, ControlError> {
    let target = detached_target(request)
        .ok_or_else(|| ControlError::rejected("session.reattach requires a detached session's id"))?;
    let (window, id) = resolve_detached(core, request, target, DetachedKind::Session)?;
    core.reattach_session(window, id).map_err(ControlError::rejected)?;
    Ok(ControlResult::id(uuid_string(id)))
}

/// `workspace.detach`: the target workspace (default `active`) of a remote window.
pub(crate) fn detach_workspace(core: &mut Core, request: &Request) -> Result<Option<ControlResult>, ControlError> {
    let window = args_of(request).window.as_deref();
    let found = Resolver::new(core.library()).workspace(request.target.as_deref(), window)?;
    if !core.remote.viewer.catalogs.contains_key(&found.window) {
        return Err(ControlError::rejected(remote_window_only("workspace.detach")));
    }
    core.detach_workspace(found.window, found.workspace);
    Ok(None)
}

/// `workspace.reattach`: a detached workspace's id or unique prefix.
pub(crate) fn reattach_workspace(core: &mut Core, request: &Request) -> Result<Option<ControlResult>, ControlError> {
    let target = detached_target(request)
        .ok_or_else(|| ControlError::rejected("workspace.reattach requires a detached workspace's id"))?;
    let (window, id) = resolve_detached(core, request, target, DetachedKind::Workspace)?;
    core.reattach_workspace(window, id).map_err(ControlError::rejected)?;
    Ok(Some(ControlResult::id(uuid_string(id))))
}

/// A reattach's id or prefix; a hidden row is never `active`.
fn detached_target(request: &Request) -> Option<&str> {
    trimmed(request.target.as_deref()).filter(|target| !target.eq_ignore_ascii_case(ACTIVE))
}

/// Finds a detached `target` among the open remote windows, or in the `--window` one alone (upstream's
/// `resolveDetached`).
fn resolve_detached(
    core: &Core,
    request: &Request,
    target: &str,
    kind: DetachedKind,
) -> Result<(Uuid, Uuid), ControlError> {
    let scope = match trimmed(args_of(request).window.as_deref()) {
        Some(window) => Some(Resolver::new(core.library()).window_id(Some(window))?),
        None => None,
    };
    let mut candidates: Vec<(Uuid, Uuid)> = Vec::new();
    for window in core.library().open_remote_windows() {
        let Some(store) = core
            .library()
            .store(window)
            .filter(|_| scope.is_none_or(|scope| scope == window))
        else {
            continue;
        };
        let detached = match kind {
            DetachedKind::Session => &store.catalog_detached_sessions,
            DetachedKind::Workspace => &store.catalog_detached_workspaces,
        };
        candidates.extend(detached.iter().map(|id| (*id, window)));
    }
    let ids: Vec<Uuid> = candidates.iter().map(|(id, _)| *id).collect();
    let noun = match kind {
        DetachedKind::Session => Noun::DetachedSession,
        DetachedKind::Workspace => Noun::DetachedWorkspace,
    };
    let resolution = resolve(target, &ids, None);
    if let TargetResolution::Resolved(id) = resolution
        && let Some((_, window)) = candidates.iter().find(|(candidate, _)| *candidate == id)
    {
        return Ok((*window, id));
    }
    Err(resolution_error(noun, target, resolution))
}
