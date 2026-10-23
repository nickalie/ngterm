//! `recent.list` / `recent.open` / `recent.clear`: File ▸ Open Recent over the socket (upstream's
//! `ControlDispatcher+Recent.swift`, `ControlRecent.swift` and the app's `ControlServer+Recent.swift`).
//!
//! The list is app-global, so `recent.list` and `recent.clear` refuse a target or `--window` before anything runs.
//! `recent.open` restores through the menu's path ([`WindowLibrary::reopen_recent_closed_outcome`]): a session still
//! pending its close is undone in place, and the window holding the entry's session or workspace wins over the
//! requested one; the reply names the window it went to. It neither raises nor focuses that window.
//!
//! `recent.open`'s `complete` and `recent.clear`'s ok are read from the list reloaded from `recent-closed.json`, never
//! from the call: a write that fails is logged and the entry stays on disk.
//!
//! [`WindowLibrary::reopen_recent_closed_outcome`]: crate::model::WindowLibrary::reopen_recent_closed_outcome

use ngterm_proto::command::RecentCmd;
use ngterm_proto::{Command, ControlRecentItem, ControlRecentOpen, ControlResult, Request};
use uuid::Uuid;

use crate::Core;
use crate::error::{ControlError, Noun};
use crate::model::RecentClosedReopen;
use crate::pane_env::uuid_string;
use crate::persist::recent_closed::reference_date;
use crate::persist::{RecentClosedItem, RecentClosedKind};
use crate::resolve::{Resolver, TargetResolution, resolution_error, resolve};

/// The refusal of `recent.open` over an empty list, and `ngtermctl recent list`'s line for one (upstream's
/// `RecentClosedTarget.emptyError`).
pub const NO_RECENT_ITEMS: &str = "no recently closed items";

/// The Unix epoch's distance from Foundation's reference date, in seconds (`timeIntervalSince1970` minus
/// `timeIntervalSinceReferenceDate`).
const UNIX_TO_REFERENCE: f64 = 978_307_200.0;

/// Routes a `recent.*` request; `None` for a bare ok.
pub(crate) fn dispatch(
    core: &mut Core,
    cmd: RecentCmd,
    request: &Request,
) -> Result<Option<ControlResult>, ControlError> {
    let window = request.args.as_ref().and_then(|args| args.window.as_deref());
    match cmd {
        RecentCmd::Open => core.open_recent_closed(request.target.as_deref(), window).map(Some),
        RecentCmd::List | RecentCmd::Clear => {
            if request.target.is_some() || window.is_some() {
                return Err(ControlError::rejected(format!(
                    "{} takes no target or --window",
                    Command::Recent(cmd).wire_name()
                )));
            }
            if cmd == RecentCmd::List {
                Ok(Some(ControlResult {
                    recent: Some(core.library.recent_closed_items().iter().map(project).collect()),
                    ..ControlResult::default()
                }))
            } else {
                core.clear_recent_closed().map(|()| None)
            }
        }
    }
}

/// One entry as `recent.list` reports it (upstream's `ControlRecentItem.init(_:)`): a session entry names its parent
/// workspace and its one session, a workspace entry itself and every session of its snapshot.
#[must_use]
pub fn project(item: &RecentClosedItem) -> ControlRecentItem {
    let (workspace, sessions): (Option<Uuid>, Vec<Uuid>) = match item.kind {
        RecentClosedKind::Session => (
            item.session.as_ref().map(|recent| recent.workspace_id),
            item.session.iter().map(|recent| recent.snapshot.id).collect(),
        ),
        RecentClosedKind::Workspace => (
            item.workspace.as_ref().map(|recent| recent.snapshot.id),
            item.workspace
                .iter()
                .flat_map(|recent| recent.snapshot.sessions.iter().map(|session| session.id))
                .collect(),
        ),
    };
    ControlRecentItem {
        id: uuid_string(item.id),
        kind: match item.kind {
            RecentClosedKind::Session => "session",
            RecentClosedKind::Workspace => "workspace",
        }
        .to_owned(),
        title: item.title.clone(),
        subtitle: item.subtitle.clone(),
        closed_at: reference_date::seconds(item.closed_at) + UNIX_TO_REFERENCE,
        workspace_id: workspace.map(uuid_string).unwrap_or_default(),
        session_ids: sessions.into_iter().map(uuid_string).collect(),
    }
}

/// The entry a `recent.open` target names (upstream's `RecentClosedTarget.resolve`): the newest without one, else the
/// one whose id matches exactly or by unique prefix, in any case. `active` names nothing: the list has no current
/// entry.
///
/// # Errors
///
/// The list is empty (whatever the target), or the target matches no entry or several.
pub fn resolve_recent_target(target: Option<&str>, items: &[RecentClosedItem]) -> Result<Uuid, ControlError> {
    let first = items.first().ok_or_else(|| ControlError::rejected(NO_RECENT_ITEMS))?;
    let Some(target) = target else {
        return Ok(first.id);
    };
    let ids: Vec<Uuid> = items.iter().map(|item| item.id).collect();
    match resolve(target, &ids, None) {
        TargetResolution::Resolved(id) => Ok(id),
        other => Err(resolution_error(Noun::RecentItem, target, other)),
    }
}

impl Core {
    /// `recent.open`: resolves the `--window` placement first, so an unknown or closed window restores nothing, then
    /// the entry, then reopens it there unless another window holds its session or workspace.
    fn open_recent_closed(
        &mut self,
        target: Option<&str>,
        window: Option<&str>,
    ) -> Result<ControlResult, ControlError> {
        let into = Resolver::new(&self.library).open_placement_window(window)?;
        let items = self.library.recent_closed_items();
        let id = resolve_recent_target(target, &items)?;
        let reopened = items
            .iter()
            .find(|item| item.id == id)
            .zip(self.reopen_recent_closed_outcome(id, into));
        let Some((item, outcome)) = reopened else {
            return Err(ControlError::rejected(format!("could not reopen {}", uuid_string(id))));
        };
        Ok(ControlResult {
            recent_open: Some(ControlRecentOpen {
                item: project(item),
                window: uuid_string(outcome.window),
                complete: outcome.complete,
            }),
            ..ControlResult::default()
        })
    }

    /// Reopens the entry `item` into open window `into` (or the window holding it) without raising that window, and
    /// reads whether the entry left the list from `recent-closed.json` once the removal was written. `None` when
    /// nothing was restored.
    pub fn reopen_recent_closed_outcome(&mut self, item: Uuid, into: Uuid) -> Option<RecentClosedReopen> {
        let outcome = self.library.reopen_recent_closed_outcome(item, Some(into))?;
        self.sync_recent_closed();
        Some(RecentClosedReopen {
            window: outcome.window,
            complete: !self.library.recent_closed_items().iter().any(|entry| entry.id == item),
        })
    }

    /// `recent.clear`: empties the list, and fails when the reloaded list still holds entries.
    fn clear_recent_closed(&mut self) -> Result<(), ControlError> {
        self.library.clear_recent_closed_items();
        self.sync_recent_closed();
        if self.library.recent_closed_items().is_empty() {
            Ok(())
        } else {
            Err(ControlError::rejected("could not clear recent items"))
        }
    }

    /// Writes the Open Recent changes made so far and reloads the list from the file; nothing without persistence,
    /// where the in-memory list is the only one.
    fn sync_recent_closed(&mut self) {
        if let Some(persistence) = self.persistence.as_ref() {
            persistence.sync_recent_closed(&mut self.library);
        }
    }
}
