//! The origin's pane layout applied to a viewer's replicas (upstream's `AppStore+RemoteLayout.swift` and
//! `agtermApp+RemoteLayout.swift`): the replicas that already exist follow the origin's order, axis and visibility, a
//! replica the origin removed is closed once that is safe, and a pane closed or split here is never recreated or
//! reordered by a layout.

use ngterm_host::Surface;
use uuid::Uuid;

use super::frames::PresentationLayout;
use crate::Core;
use crate::model::{AppStore, Session, SplitAxis, StatusPane, SwapRefusal};

fn realized(slot: &crate::model::SurfaceSlot) -> bool {
    slot.get().is_some_and(Surface::is_realized)
}

impl AppStore {
    /// Applies `layout` to session `id`'s existing replicas (upstream's `applyRemoteLayout`): only when both panes here
    /// are realized replicas of exactly the origin's two panes does it swap, set the axis and show or hide the split.
    /// With `order` false, the panes keep their order here and only the axis, the visibility and the removals follow
    /// `layout`. The local identities a removal frame now lets close.
    fn follow_remote_layout(&mut self, layout: &PresentationLayout, id: Uuid, order: bool) -> Vec<Uuid> {
        if !layout.is_valid() {
            return Vec::new();
        }
        let Some(state) = self
            .session_mut(id)
            .and_then(|session| session.remote_presentation.as_mut())
        else {
            return Vec::new();
        };
        state.layout = Some(layout.clone());
        if let Some(swap) = self.session(id).and_then(|session| replicas_follow(session, layout)) {
            if swap && order && self.swap_panes(id).is_err() {
                return Vec::new();
            }
            if let Some(axis) = layout.axis.as_deref().and_then(SplitAxis::from_wire)
                && let Some(session) = self.session_mut(id)
            {
                session.split.axis = axis;
            }
            self.set_split_visibility(id, layout.shown, None);
        }
        let Some(session) = self.session(id) else {
            return Vec::new();
        };
        [session.split_pane_identity, Some(session.pane_identity)]
            .into_iter()
            .flatten()
            .filter(|local| self.can_close_removed_remote_pane(*local, id))
            .collect()
    }

    /// Whether a confirmed removal on the origin lets this replica close without discarding a pending local split
    /// (upstream's `canCloseRemovedRemotePane`).
    #[must_use]
    pub fn can_close_removed_remote_pane(&self, local: Uuid, id: Uuid) -> bool {
        let Some(session) = self.session(id) else {
            return false;
        };
        let Some(state) = session.remote_presentation() else {
            return false;
        };
        let (Some(layout), Some(remote)) = (state.layout.as_ref(), state.binding.remote_pane(local)) else {
            return false;
        };
        if session.pane_role_for_identity(local).is_none() || layout.panes.contains(&remote) {
            return false;
        }
        // the ordinary primary close would discard a local split that has not produced its terminal yet
        if local == session.pane_identity
            && let Some(split) = session.split_pane_identity
            && state.binding.remote_pane(split).is_none()
            && !realized(&session.split_surface)
        {
            return false;
        }
        true
    }

    /// Records that the ssh of the mapped replica `local` ended and its terminal holds, for a later removal frame
    /// (upstream's `remotePaneHeld`).
    pub fn remote_pane_held(&mut self, local: Uuid, id: Uuid) {
        let Some(session) = self.session_mut(id) else {
            return;
        };
        if session.pane_role_for_identity(local).is_none() {
            return;
        }
        if let Some(state) = session.remote_presentation.as_mut()
            && state.binding.remote_pane(local).is_some()
        {
            state.held_panes.insert(local);
        }
    }

    /// Whether a held exit was recorded for the replica `local`.
    #[must_use]
    pub fn remote_pane_is_held(&self, local: Uuid, id: Uuid) -> bool {
        self.session(id)
            .and_then(Session::remote_presentation)
            .is_some_and(|state| state.held_panes.contains(&local))
    }

    /// The held replica `local` attached again.
    pub fn remote_pane_resumed(&mut self, local: Uuid, id: Uuid) {
        if let Some(state) = self
            .session_mut(id)
            .and_then(|session| session.remote_presentation.as_mut())
        {
            state.held_panes.remove(&local);
        }
    }
}

/// `Some(swap)` when both panes are realized replicas of exactly the layout's two panes, `swap` when the layout's
/// primary is the local split; `None` otherwise, which leaves the arrangement alone.
fn replicas_follow(session: &Session, layout: &PresentationLayout) -> Option<bool> {
    let split = session.split_pane_identity.filter(|_| session.split.exists)?;
    if !realized(&session.surface) || !realized(&session.split_surface) {
        return None;
    }
    let binding = &session.remote_presentation()?.binding;
    let primary_origin = binding.remote_pane(session.pane_identity)?;
    let split_origin = binding.remote_pane(split)?;
    let mut wanted = layout.panes.clone();
    wanted.sort_unstable();
    let mut have = vec![primary_origin, split_origin];
    have.sort_unstable();
    (wanted == have).then(|| binding.local_pane(layout.primary) == Some(split))
}

fn has_unbound_split(session: &Session) -> bool {
    let Some(binding) = session.remote_presentation().map(|state| &state.binding) else {
        return false;
    };
    session.split.exists
        && [Some(session.pane_identity), session.split_pane_identity]
            .into_iter()
            .any(|pane| pane.is_none_or(|pane| binding.remote_pane(pane).is_none()))
}

impl Core {
    /// Swaps session `id`'s panes for the user. The swap is newer than the order of a layout cached for a pane still
    /// to be realized, so that layout's reconcile keeps the panes in their order here.
    ///
    /// # Errors
    ///
    /// The refusal, with nothing changed.
    pub(crate) fn swap_panes_here(&mut self, id: Uuid) -> Result<(), SwapRefusal> {
        self.library_mut()
            .store_for_session_mut(id)
            .ok_or(SwapRefusal::NoSession)?
            .swap_panes(id)?;
        if self.cached_layout_pending(id) {
            self.remote.viewer.swapped_here.insert(id);
        }
        Ok(())
    }

    /// Whether a layout cached for session `id` is still to be reconciled: a bound pane waits for its realization, or
    /// a split pane waits for its origin pane.
    pub(super) fn cached_layout_pending(&self, id: Uuid) -> bool {
        self.remote.viewer.split_layouts.values().any(|session| *session == id)
            || self.library().held_session(id).is_some_and(has_unbound_split)
    }

    /// Applies the origin's layout to session `id`'s replicas and closes each one the origin removed (upstream's
    /// `agtermApp.applyRemoteLayout`). A new layout from the origin is newer than a swap made here, except the origin's
    /// answer to the split requested here, in the order from before that swap, while the layout of the session is
    /// pending.
    pub(crate) fn apply_remote_layout(&mut self, layout: &PresentationLayout, id: Uuid) {
        let answer = layout.shown && layout.panes.len() == 2 && self.remote.viewer.awaited_split_answers.remove(&id);
        if !(answer && self.layout_has_the_order_before_the_swap(layout, id)) {
            self.remote.viewer.swapped_here.remove(&id);
        }
        self.apply_cached_layout(layout, id);
    }

    fn layout_has_the_order_before_the_swap(&self, layout: &PresentationLayout, id: Uuid) -> bool {
        self.cached_layout_pending(id)
            && self.library().held_session(id).is_some_and(|session| {
                session.remote_presentation().is_some_and(|state| {
                    let primary = state.binding.local_pane(layout.primary);
                    primary.is_some() && primary == session.split_pane_identity
                })
            })
    }

    /// Applies the layout cached for session `id`, in the order here when the user swapped the panes while a layout of
    /// the session was still pending.
    pub(super) fn apply_cached_layout(&mut self, layout: &PresentationLayout, id: Uuid) {
        let order = !self.remote.viewer.swapped_here.contains(&id);
        self.follow_remote_layout(layout, id, order);
    }

    fn follow_remote_layout(&mut self, layout: &PresentationLayout, id: Uuid, order: bool) {
        let removed = self
            .library_mut()
            .store_for_session_mut(id)
            .map(|store| store.follow_remote_layout(layout, id, order))
            .unwrap_or_default();
        for local in removed {
            self.close_removed_remote_pane(local, id);
        }
    }

    /// The ssh of a replica's pane ended and its terminal holds (upstream's `handleRemotePaneHeld`): recorded, and the
    /// pane closes now when the origin already removed it.
    pub fn remote_pane_held(&mut self, session: Uuid, token: &str) -> bool {
        let Ok(local) = Uuid::parse_str(token) else {
            return false;
        };
        let Some(store) = self.library_mut().store_for_session_mut(session) else {
            return false;
        };
        store.remote_pane_held(local, session);
        self.close_removed_remote_pane(local, session)
    }

    /// Closes the replica `local` the origin removed, when that is safe (upstream's `closeRemovedRemotePane`): a split
    /// with no terminal yet closes outright; otherwise the last replica keeps its terminal until its ssh ends, so no
    /// empty row needs replacing. Whether it closed.
    fn close_removed_remote_pane(&mut self, local: Uuid, id: Uuid) -> bool {
        let Some(store) = self.library_mut().store_for_session_mut(id) else {
            return false;
        };
        if !store.can_close_removed_remote_pane(local, id) {
            return false;
        }
        let held = store.remote_pane_is_held(local, id);
        let Some(session) = store.session(id) else {
            return false;
        };
        let split = session.split_pane_identity == Some(local);
        let (slot, survivor) = if split {
            (&session.split_surface, &session.surface)
        } else {
            (&session.surface, &session.split_surface)
        };
        let Some(token) = slot.get().map(|surface| surface.pane_token().to_owned()) else {
            if split {
                store.close_split(id);
                return true;
            }
            return false;
        };
        if !realized(survivor) && !held {
            return false;
        }
        let role = session.pane_role_for_token(&token);
        if role == Some(StatusPane::Scratch) || role.is_none() {
            return false;
        }
        self.pane_exited(id, &token)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ngterm_host::fakes::FakeSurface;

    use super::*;
    use crate::live::daemon_name;
    use crate::model::{NewSession, RemoteBinding};

    struct Attached {
        store: AppStore,
        id: Uuid,
        origin_a: Uuid,
        origin_b: Uuid,
    }

    fn fake(identity: Uuid, realized: bool) -> Box<dyn ngterm_host::Surface> {
        Box::new(FakeSurface::new(crate::pane_env::uuid_string(identity)).with_realized(realized))
    }

    fn attached() -> Attached {
        let mut store = AppStore::new();
        let workspace = store.add_workspace("work");
        let id = store
            .add_session(
                workspace,
                NewSession {
                    remote_host: Some("origin".to_owned()),
                    ..NewSession::at_cwd("/tmp")
                },
            )
            .unwrap();
        store.toggle_split(id, None);
        let session = store.session_mut(id).unwrap();
        let (primary, split) = (session.pane_identity, session.split_pane_identity.unwrap());
        session.surface.replace(fake(primary, true));
        session.split_surface.replace(fake(split, true));
        let (origin_a, origin_b) = (Uuid::new_v4(), Uuid::new_v4());
        let daemons = HashMap::from([(primary, daemon_name(origin_a)), (split, daemon_name(origin_b))]);
        store.bind_remote(RemoteBinding::new("ORIGIN", daemons, Some(1), None), id);
        Attached {
            store,
            id,
            origin_a,
            origin_b,
        }
    }

    fn layout(panes: Vec<Uuid>, primary: Uuid, axis: Option<&str>, shown: bool) -> PresentationLayout {
        PresentationLayout {
            panes,
            primary,
            axis: axis.map(str::to_owned),
            shown,
        }
    }

    #[test]
    fn a_layout_reorders_only_existing_replicas_and_keeps_focus_on_its_pane() {
        let mut a = attached();
        let session = a.store.session_mut(a.id).unwrap();
        let (primary, split) = (session.pane_identity, session.split_pane_identity.unwrap());
        session.split.focused = false;
        session.split.ratio = Some(0.35);
        let swapped = layout(vec![a.origin_b, a.origin_a], a.origin_b, Some("horizontal"), false);
        assert!(a.store.follow_remote_layout(&swapped, a.id, true).is_empty());
        let session = a.store.session(a.id).unwrap();
        assert_eq!(session.pane_identity, split);
        assert_eq!(session.split_pane_identity, Some(primary));
        assert!(session.split.focused);
        assert_eq!(session.split.ratio, Some(0.35));
        assert_eq!(session.split.axis, SplitAxis::from_wire("horizontal").unwrap());
        assert!(!session.split.shown);
        assert!(session.split.exists);
        let removal = layout(vec![a.origin_b], a.origin_b, None, false);
        assert_eq!(a.store.follow_remote_layout(&removal, a.id, true), [primary]);
    }

    #[test]
    fn a_closed_replica_is_never_recreated() {
        let mut a = attached();
        a.store.close_split(a.id);
        let both = layout(vec![a.origin_a, a.origin_b], a.origin_a, Some("horizontal"), true);
        assert!(a.store.follow_remote_layout(&both, a.id, true).is_empty());
        let session = a.store.session(a.id).unwrap();
        assert!(!session.split.exists);
        assert_eq!(session.split_pane_identity, None);
        assert!(!session.split_surface.is_present());
    }

    #[test]
    fn a_local_split_is_neither_reordered_nor_hidden() {
        for pending in [false, true] {
            let mut a = attached();
            a.store.close_split(a.id);
            a.store.toggle_split(a.id, None);
            if !pending {
                let session = a.store.session_mut(a.id).unwrap();
                let split = session.split_pane_identity.unwrap();
                session.split_surface.replace(fake(split, true));
            }
            let session = a.store.session(a.id).unwrap();
            let (local, primary, axis) = (session.split_pane_identity, session.pane_identity, session.split.axis);
            let swapped = layout(vec![a.origin_b, a.origin_a], a.origin_b, Some("horizontal"), false);
            assert!(a.store.follow_remote_layout(&swapped, a.id, true).is_empty());
            let session = a.store.session(a.id).unwrap();
            assert_eq!(session.split_pane_identity, local);
            assert_eq!(session.pane_identity, primary);
            assert!(session.split.shown);
            assert_eq!(session.split.axis, axis);
            let removed = layout(vec![a.origin_b], a.origin_b, None, false);
            a.store.follow_remote_layout(&removed, a.id, true);
            assert_eq!(
                a.store.can_close_removed_remote_pane(primary, a.id),
                !pending,
                "pending: {pending}"
            );
            assert!(!a.store.can_close_removed_remote_pane(local.unwrap(), a.id));
        }
    }

    #[test]
    fn an_unrealized_replica_is_not_shown_by_a_layout() {
        let mut a = attached();
        let session = a.store.session_mut(a.id).unwrap();
        let split = session.split_pane_identity.unwrap();
        session.split_surface.replace(fake(split, false));
        a.store.set_split_visibility(a.id, false, None);
        let shown = layout(vec![a.origin_a, a.origin_b], a.origin_a, Some("vertical"), true);
        a.store.follow_remote_layout(&shown, a.id, true);
        assert!(!a.store.session(a.id).unwrap().split.shown);
    }

    /// Upstream's `aLocalPaneHeldExitIsIgnoredByTheRemoteHandlers`.
    #[test]
    fn a_local_pane_held_exit_is_ignored_by_the_remote_handlers() {
        let mut store = AppStore::new();
        let workspace = store.add_workspace("work");
        let spec = crate::model::NewSession {
            command: Some("sleep 1".to_owned()),
            wait: true,
            ..crate::model::NewSession::at_cwd("/tmp")
        };
        let id = store.add_session(workspace, spec).unwrap();
        let pane = store.session(id).unwrap().pane_identity;

        store.remote_pane_held(pane, id);

        assert!(!store.remote_pane_is_held(pane, id));
        assert!(!store.can_close_removed_remote_pane(pane, id));
        assert!(store.session(id).is_some());
    }

    #[test]
    fn a_replica_that_attached_again_is_no_longer_held() {
        let mut a = attached();
        let primary = a.store.session(a.id).unwrap().pane_identity;
        a.store.remote_pane_held(primary, a.id);
        assert!(a.store.remote_pane_is_held(primary, a.id));
        a.store.remote_pane_resumed(primary, a.id);
        assert!(!a.store.remote_pane_is_held(primary, a.id));
    }

    fn core_with(a: Attached) -> (Core, Uuid) {
        let mut library = crate::model::WindowLibrary::new("/tmp");
        let window = library.new_window(None);
        *library.store_mut(window).unwrap() = a.store;
        let core = Core::new(
            crate::CoreConfig {
                app: ngterm_proto::AppIdentity::from_recorded("0.1.0", None),
                capabilities: Vec::new(),
            },
            crate::CoreState {
                library,
                settings: crate::persist::AppSettings::default(),
                persistence: None,
            },
        );
        (core, a.id)
    }

    #[test]
    fn an_invalid_layout_cannot_replace_the_last_confirmed_layout() {
        let mut a = attached();
        let good = layout(vec![a.origin_a, a.origin_b], a.origin_a, Some("vertical"), true);
        a.store.follow_remote_layout(&good, a.id, true);
        assert!(
            a.store
                .follow_remote_layout(&PresentationLayout::invalid(), a.id, true)
                .is_empty()
        );
        assert_eq!(
            a.store.session(a.id).unwrap().remote_presentation().unwrap().layout,
            Some(good.clone())
        );
        assert!(a.store.follow_remote_layout(&good, Uuid::new_v4(), true).is_empty());
        let missing = Uuid::new_v4();
        assert!(!a.store.can_close_removed_remote_pane(missing, missing));
        a.store.remote_pane_held(missing, missing);
        a.store.remote_pane_held(missing, a.id);
        assert!(!a.store.remote_pane_is_held(missing, a.id));
    }

    #[test]
    fn a_removed_split_without_a_terminal_closes_immediately() {
        let mut a = attached();
        drop(a.store.session_mut(a.id).unwrap().split_surface.take());
        let removal = layout(vec![a.origin_a], a.origin_a, None, false);
        let (mut core, id) = core_with(a);
        core.apply_remote_layout(&removal, id);
        let row = core.library().held_session(id).unwrap();
        assert!(!row.split.exists);
        assert!(row.split_pane_identity.is_none());
        assert!(row.surface.is_present());
    }

    #[test]
    fn the_last_terminal_waits_for_ssh_to_end_after_origin_removal() {
        let mut a = attached();
        a.store.close_split(a.id);
        let primary = a.store.session(a.id).unwrap().pane_identity;
        let removal = layout(vec![a.origin_b], a.origin_b, None, false);
        let (mut core, id) = core_with(a);
        core.apply_remote_layout(&removal, id);
        assert!(core.library().held_session(id).unwrap().surface.is_present());
        assert!(core.remote_pane_held(id, &crate::pane_env::uuid_string(primary)));
        assert!(core.library().store_for_session(id).is_none());
    }

    #[test]
    fn a_removed_primary_without_a_terminal_keeps_the_row() {
        let mut a = attached();
        drop(a.store.session_mut(a.id).unwrap().surface.take());
        let primary = a.store.session(a.id).unwrap().pane_identity;
        let removal = layout(vec![a.origin_b], a.origin_b, None, false);
        let (mut core, id) = core_with(a);
        core.apply_remote_layout(&removal, id);
        assert!(!core.remote_pane_held(id, &crate::pane_env::uuid_string(primary)));
        assert!(core.library().held_session(id).is_some());
        assert!(!core.remote_pane_held(id, "bad token"));
        assert!(!core.remote_pane_held(Uuid::new_v4(), &crate::pane_env::uuid_string(primary)));
    }

    fn swapped_here_with_a_cached_origin_swap(split_realized: bool) -> (Core, Uuid, Uuid, Uuid) {
        let mut a = attached();
        let cached = layout(vec![a.origin_b, a.origin_a], a.origin_b, Some("horizontal"), true);
        let session = a.store.session_mut(a.id).unwrap();
        let (primary, split) = (session.pane_identity, session.split_pane_identity.unwrap());
        session.split_surface.replace(fake(split, split_realized));
        session.remote_presentation.as_mut().unwrap().layout = Some(cached);
        let (mut core, id) = core_with(a);
        core.remote.viewer.swapped_here.insert(id);
        (core, id, primary, split)
    }

    fn assert_order_kept_with_origin_axis(core: &Core, id: Uuid, primary: Uuid, split: Uuid) {
        let row = core.library().held_session(id).unwrap();
        assert_eq!((row.pane_identity, row.split_pane_identity), (primary, Some(split)));
        assert_eq!(row.split.axis, SplitAxis::TopBottom);
        assert!(core.remote.viewer.swapped_here.is_empty());
    }

    #[test]
    fn a_swap_here_holds_for_every_pending_entry_of_its_session() {
        let (mut core, id, primary, split) = swapped_here_with_a_cached_origin_swap(true);
        core.remote.viewer.split_layouts.extend([(primary, id), (split, id)]);
        core.advance_remote_splits();
        assert_order_kept_with_origin_axis(&core, id, primary, split);
    }

    #[test]
    fn a_dropped_entry_keeps_the_swap_mark_for_an_entry_still_pending() {
        let (mut core, id, primary, split) = swapped_here_with_a_cached_origin_swap(false);
        core.remote
            .viewer
            .split_layouts
            .extend([(Uuid::new_v4(), id), (primary, id)]);
        core.advance_remote_splits();
        assert!(core.remote.viewer.swapped_here.contains(&id));
        let row = core.library_mut().held_session_mut(id).unwrap();
        row.split_surface.replace(fake(split, true));
        core.advance_remote_splits();
        assert_order_kept_with_origin_axis(&core, id, primary, split);
    }

    #[test]
    fn a_swap_here_before_the_split_binds_keeps_its_order_at_the_reconcile() {
        let mut a = attached();
        let session = a.store.session(a.id).unwrap();
        let (primary, split) = (session.pane_identity, session.split_pane_identity.unwrap());
        let daemons = HashMap::from([(primary, daemon_name(a.origin_a))]);
        a.store
            .bind_remote(RemoteBinding::new("ORIGIN", daemons, Some(1), None), a.id);
        let cached = layout(vec![a.origin_a, a.origin_b], a.origin_a, Some("horizontal"), true);
        a.store
            .session_mut(a.id)
            .unwrap()
            .remote_presentation
            .as_mut()
            .unwrap()
            .layout = Some(cached);
        let origin_b = a.origin_b;
        let (mut core, id) = core_with(a);
        core.swap_panes_here(id).unwrap();
        core.advance_remote_splits();
        assert!(core.remote.viewer.swapped_here.contains(&id));
        let row = core.library_mut().held_session_mut(id).unwrap();
        let state = row.remote_presentation.as_mut().unwrap();
        state.binding.insert_pane(split, daemon_name(origin_b));
        core.remote.viewer.split_layouts.insert(split, id);
        core.advance_remote_splits();
        assert_order_kept_with_origin_axis(&core, id, split, primary);
    }

    fn swapped_here_before_the_split_binds() -> (Core, Uuid, Uuid, Uuid, Uuid, Uuid) {
        let mut a = attached();
        let session = a.store.session(a.id).unwrap();
        let (primary, split) = (session.pane_identity, session.split_pane_identity.unwrap());
        let daemons = HashMap::from([(primary, daemon_name(a.origin_a))]);
        a.store
            .bind_remote(RemoteBinding::new("ORIGIN", daemons, Some(1), None), a.id);
        let (origin_a, origin_b) = (a.origin_a, a.origin_b);
        let (mut core, id) = core_with(a);
        core.remote.viewer.awaited_split_answers.insert(id);
        core.swap_panes_here(id).unwrap();
        (core, id, primary, split, origin_a, origin_b)
    }

    fn bind_split_and_reconcile(core: &mut Core, id: Uuid, split: Uuid, origin: Uuid) {
        let row = core.library_mut().held_session_mut(id).unwrap();
        let state = row.remote_presentation.as_mut().unwrap();
        state.binding.insert_pane(split, daemon_name(origin));
        core.remote.viewer.split_layouts.insert(split, id);
        core.advance_remote_splits();
    }

    #[test]
    fn the_origin_layout_of_the_requested_split_keeps_a_swap_made_before_the_bind() {
        let (mut core, id, primary, split, origin_a, origin_b) = swapped_here_before_the_split_binds();
        let requested = layout(vec![origin_a, origin_b], origin_a, Some("horizontal"), true);
        core.apply_remote_layout(&requested, id);
        assert!(core.remote.viewer.swapped_here.contains(&id));
        bind_split_and_reconcile(&mut core, id, split, origin_b);
        assert_order_kept_with_origin_axis(&core, id, split, primary);
    }

    fn bind_split_unrealized(core: &mut Core, id: Uuid, split: Uuid, origin: Uuid) {
        let row = core.library_mut().held_session_mut(id).unwrap();
        row.split_surface.replace(fake(split, false));
        bind_split_and_reconcile(core, id, split, origin);
    }

    #[test]
    fn the_origin_layout_of_the_requested_split_after_the_bind_keeps_a_swap_made_before_it() {
        let (mut core, id, primary, split, origin_a, origin_b) = swapped_here_before_the_split_binds();
        bind_split_unrealized(&mut core, id, split, origin_b);
        let requested = layout(vec![origin_a, origin_b], origin_a, Some("horizontal"), true);
        core.apply_remote_layout(&requested, id);
        assert!(core.remote.viewer.swapped_here.contains(&id));
        let row = core.library_mut().held_session_mut(id).unwrap();
        row.split_surface.replace(fake(split, true));
        core.advance_remote_splits();
        assert_order_kept_with_origin_axis(&core, id, split, primary);
    }

    #[test]
    fn an_origin_layout_with_another_primary_after_the_bind_clears_the_swap_mark() {
        let (mut core, id, _, split, origin_a, origin_b) = swapped_here_before_the_split_binds();
        bind_split_unrealized(&mut core, id, split, origin_b);
        let reordered = layout(vec![origin_b, origin_a], origin_b, Some("horizontal"), true);
        core.apply_remote_layout(&reordered, id);
        assert!(core.remote.viewer.swapped_here.is_empty());
    }

    #[test]
    fn an_origin_layout_with_another_primary_before_the_bind_clears_the_swap_mark() {
        let (mut core, id, _, split, origin_a, origin_b) = swapped_here_before_the_split_binds();
        let reordered = layout(vec![origin_b, origin_a], origin_b, Some("horizontal"), true);
        core.apply_remote_layout(&reordered, id);
        assert!(core.remote.viewer.swapped_here.is_empty());
        bind_split_and_reconcile(&mut core, id, split, origin_b);
        let row = core.library().held_session(id).unwrap();
        assert_eq!(row.pane_identity, split);
    }
}
