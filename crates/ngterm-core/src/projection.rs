//! The `tree` projection of one window's store (upstream's `AppStore.controlTree`, `ControlProjection.swift`).
//!
//! Built fresh per request from the model. What the model does not hold yet reads as upstream reports it in that
//! state: overlays, asks, the dashboard, zoom, the quick terminal and live sessions arrive with their command
//! groups, and the live foreground-process read with the host seam that answers it.

use std::time::{SystemTime, UNIX_EPOCH};

use ngterm_proto::AppIdentity;
use ngterm_proto::tree::{
    ControlHtmlOverlayNode, ControlHudNode, ControlRemoteOverlayNode, ControlSessionAsk, ControlSessionNode,
    ControlSurfaceNode, ControlTree, ControlWorkspaceNode,
};
use uuid::Uuid;

use crate::html::{HtmlOverlay, HtmlSource};
use crate::hud::HudSpinner;
use crate::model::{
    AgentStatus, AppStore, OverlayPane, RemotePresentationState, Session, StatusPane, SurfaceSlot, Workspace,
};
use crate::pane_env::uuid_string;
use crate::persist::AppSettings;
use crate::zoom::{SurfaceId, ZoomSurface};

/// The app-wide inputs of a tree beside the window's store.
#[derive(Debug, Clone, Copy)]
pub struct TreeContext<'a> {
    /// The serving app (`tree.app`).
    pub app: &'a AppIdentity,
    /// The flagged-view layout, link mode and auto-follow timeout are app settings.
    pub settings: &'a AppSettings,
    /// The last `windows.json` write failed.
    pub index_unsaved: bool,
    /// Whether the quick terminal is shown, app-wide.
    pub quick_visible: bool,
}

/// The tree of `store`.
#[must_use]
pub fn control_tree(store: &AppStore, context: &TreeContext<'_>) -> ControlTree {
    let current = store.current_workspace_id();
    let selected = store.selected_session_id();
    let settings = context.settings;
    let mut workspaces: Vec<ControlWorkspaceNode> = store
        .workspaces()
        .iter()
        .map(|workspace| workspace_node(store, workspace, current, selected))
        .collect();
    // one app-wide page zoom, so every page reports the same factor
    let zoom = crate::html::zoom::resolve(settings.html_overlay_zoom);
    let pages = workspaces
        .iter_mut()
        .flat_map(|workspace| workspace.sessions.iter_mut())
        .filter_map(|session| session.html_overlays.as_mut())
        .flatten();
    // a page shown on the presenting machine has that machine's zoom, which is not reported
    for page in pages.filter(|page| page.remote.is_none()) {
        page.zoom = Some(zoom);
    }
    let sidebar = store.sidebar();
    ControlTree {
        workspaces,
        auto_follow_ms: settings
            .effective_auto_follow_attention()
            .timeout_secs()
            .map(|secs| i64::from(secs) * 1000),
        attention_includes_active: Some(settings.attention_includes_active.unwrap_or(false)),
        sidebar_visible: Some(sidebar.visible),
        sidebar_mode: Some(sidebar.mode.wire_name().to_owned()),
        sidebar_flagged_layout: Some(settings.effective_flagged_view_layout().raw().to_owned()),
        link_open_mode: Some(settings.effective_link_open_mode().raw().to_owned()),
        sidebar_width: Some(sidebar.width),
        workspace_filter: Some(store.focus().enabled()),
        quick_visible: Some(context.quick_visible),
        app: Some(context.app.clone()),
        index_unsaved: context.index_unsaved.then_some(true),
        ..ControlTree::default()
    }
}

fn workspace_node(
    store: &AppStore,
    workspace: &Workspace,
    current: Option<Uuid>,
    selected: Option<Uuid>,
) -> ControlWorkspaceNode {
    let id = workspace.id();
    ControlWorkspaceNode {
        id: uuid_string(id),
        name: workspace.name.clone(),
        active: current == Some(id),
        focused: store.focus().ids().contains(&id).then_some(true),
        collapsed: (!workspace.is_expanded).then_some(true),
        tint: workspace.tint.map(|tint| tint.color().wire_name().to_owned()),
        tint_on: workspace.tint.map(|tint| tint.parts().wire_names()),
        sessions: workspace
            .sessions
            .iter()
            .map(|session| session_node(session, selected == Some(session.id())))
            .collect(),
    }
}

/// One session's node.
#[must_use]
pub fn session_node(session: &Session, active: bool) -> ControlSessionNode {
    let mut node = ControlSessionNode::new(
        uuid_string(session.id()),
        session.display_name(),
        session.effective_cwd().to_owned(),
    );
    node.title.clone_from(&session.osc_title);
    node.active = active;
    node.scratch = session.scratch_active;
    node.flagged = session.flagged;
    node.context = session.effective_context().map(str::to_owned);
    node.remote_host = session.remote_host().map(str::to_owned);
    node.presentation = session.remote_presentation().map(RemotePresentationState::node);
    node.unseen = (session.unseen_count > 0).then(|| i64::try_from(session.unseen_count).unwrap_or(i64::MAX));
    node.command_wait = session.launch.holds().then_some(true);
    node.split_command_wait = session.split_launch.holds().then_some(true);
    node.restore_command.clone_from(&session.restore.command);
    node.split_restore_command.clone_from(&session.split_restore.command);
    node.background = session
        .background_watermark
        .as_ref()
        .and_then(|watermark| serde_json::to_value(watermark).ok());
    node.pane_backgrounds = (!session.pane_backgrounds.is_empty())
        .then(|| serde_json::to_value(&session.pane_backgrounds).ok())
        .flatten();
    node.status_changed_at = session.status_changed_at.map(epoch_seconds);
    node.last_selected_at = session.last_selected_at.map(epoch_seconds);
    node.tint = session.tint.map(|tint| tint.color().wire_name().to_owned());
    node.tint_on = session.tint.map(|tint| tint.parts().wire_names());
    node.backed_by_live = Some(all_panes_backed(session));
    node.surfaces = Some(surface_nodes(session));
    node.realized = Some(session.surface.get().is_some_and(ngterm_host::Surface::is_realized));
    node.font_size = font_size(session.pane_slot(session.addressable_pane()));
    node.split_font_size = font_size(&session.split_surface);
    node.scratch_font_size = font_size(&session.scratch_surface);
    apply_split(&mut node, session);
    apply_status(&mut node, session);
    apply_overlay(&mut node, session);
    node
}

/// The overlay read-back: `overlay` and its size for a caller's program, `hud` for a HUD, never both; `paneOverlays`
/// names the panes covered by their own overlay, omitted when neither is. Both report occupancy, a hidden overlay
/// included; `hiddenOverlays` names the hidden slots.
fn apply_overlay(node: &mut ControlSessionNode, session: &Session) {
    let slot = &session.overlay;
    node.overlay = session.cover_overlay_active();
    node.overlay_size_percent = slot.size_percent.filter(|_| session.cover_overlay_active());
    let content = session.overlay_content_size();
    node.overlay_width = content.map(|(width, _)| width);
    node.overlay_height = content.map(|(_, height)| height);
    let panes = session.pane_overlays.open_panes();
    node.pane_overlays = (!panes.is_empty()).then(|| panes.iter().map(|pane| pane.wire_name().to_owned()).collect());
    node.hud = hud_node(session);
    node.html_overlays = html_overlay_nodes(session);
    let hidden = session.hidden_overlay_names();
    node.hidden_overlays = (!hidden.is_empty()).then(|| hidden.into_iter().map(str::to_owned).collect());
    node.ask = session.ask().map(|pending| ControlSessionAsk {
        id: pending.ask.id.clone(),
        pane: session.ask_target_pane().map(|pane| pane.wire_name().to_owned()),
        remote: pending.remote_owner.map(|_| true),
        replica: pending.replica.map(|_| true),
    });
    // one entry per slot a viewer's presenter holds
    let slots = session.remote_overlays().slots();
    node.remote_overlays = (!slots.is_empty()).then(|| {
        slots
            .iter()
            .map(|slot| ControlRemoteOverlayNode {
                pane: slot.pane.map(|pane| pane.wire_name().to_owned()),
                size_percent: slot.size_percent,
                page_id: slot.page.as_ref().map(|page| uuid_string(page.overlay.id)),
            })
            .collect()
    });
}

/// The pages in `session`'s slots, session-wide first (upstream's `htmlOverlayNodes`): a local page as its web view last
/// reported it, a page shown on the presenting machine as that machine last reported it, marked `remote`; `None` when
/// no slot holds one.
fn html_overlay_nodes(session: &Session) -> Option<Vec<ControlHtmlOverlayNode>> {
    let local = session.html_overlays();
    let nodes: Vec<ControlHtmlOverlayNode> = [None, Some(OverlayPane::Left), Some(OverlayPane::Right)]
        .into_iter()
        .filter_map(|slot| {
            let name = slot.map(OverlayPane::wire_name);
            if let Some((_, page)) = local.iter().find(|(pane, _)| *pane == slot) {
                let mut node = html_overlay_node(name, page);
                node.hidden = session.overlay_hidden(slot).then_some(true);
                return Some(node);
            }
            let remote = session.remote_overlays().slot(slot)?.page.as_ref()?;
            let mut node = html_overlay_node(name, &remote.overlay);
            node.hidden = remote.hidden.then_some(true);
            node.remote = Some(true);
            Some(node)
        })
        .collect();
    (!nodes.is_empty()).then_some(nodes)
}

/// One page in an overlay slot, as upstream's `htmlOverlayNode`: the source opened (a replica's path on the machine
/// that opened it, with no grant), the load state, what the web view shows once it reported, and the flags; the zoom
/// is the app's, set by the tree.
fn html_overlay_node(slot: Option<&str>, page: &HtmlOverlay) -> ControlHtmlOverlayNode {
    let (file, cwd, url) = match &page.source {
        HtmlSource::File { path, grant_root } => (Some(path.clone()), grant_root.clone(), None),
        HtmlSource::Url(address) => (None, None, Some(address.as_str().to_owned())),
        HtmlSource::Replica { path } => (Some(path.clone()), None, None),
    };
    let current = page.current.as_ref();
    ControlHtmlOverlayNode {
        pane: slot.map(str::to_owned),
        file,
        cwd,
        url,
        state: page.load_state.wire_name().to_owned(),
        error: page.load_error.clone(),
        page: current.map(|current| current.page.clone()),
        title: current.and_then(|current| current.title.clone()),
        can_go_back: current.map(|current| current.can_go_back),
        can_go_forward: current.map(|current| current.can_go_forward),
        navigation: page.flags.navigation.then_some(true),
        javascript: page.flags.javascript,
        chromeless: page.flags.chromeless,
        persistent: page.flags.persistent,
        browse: page.flags.browse,
        zoom: None,
        id: Some(uuid_string(page.id)),
        hidden: None,
        remote: None,
        replica: page.replica_of.map(|_| true),
    }
}

/// The HUD occupying the slot, as upstream's `hudNode`: every effective value, omitted without a HUD.
fn hud_node(session: &Session) -> Option<ControlHudNode> {
    let slot = &session.overlay;
    let hud = slot.hud.as_ref().filter(|_| slot.hud_active())?;
    let spec = &hud.spec;
    Some(ControlHudNode {
        message: spec.message.clone(),
        detail: spec.detail.clone(),
        spinner: spec
            .spinner
            .map_or(HudSpinner::NONE_NAME, HudSpinner::wire_name)
            .to_owned(),
        background_color: spec.background_color.clone(),
        text_color: spec.text_color.clone(),
        size_percent: slot.size_percent,
        height_percent: Some(hud.height_percent),
        position: spec.position.wire_name().to_owned(),
        pane: session.hud_target_pane().map(|pane| pane.wire_name().to_owned()),
        hide_after: spec.effective_hide_after(),
        markdown: spec.markdown,
        font_size: spec.font_size,
        sticky: spec.sticky,
        frame: spec.frame,
    })
}

/// The split read-back, present exactly while the session has a split pane, shown or hidden.
fn apply_split(node: &mut ControlSessionNode, session: &Session) {
    let split = &session.split;
    node.split = split.shown;
    if !split.exists {
        return;
    }
    node.has_split = Some(true);
    node.split_axis = Some(split.axis.wire_name().to_owned());
    node.split_ratio = split.ratio;
    node.split_focused = Some(split.focused);
    node.split_cwd = Some(session.cwd_for(StatusPane::Right).to_owned());
}

/// The agent status read-back, absent while idle.
fn apply_status(node: &mut ControlSessionNode, session: &Session) {
    let indicator = &session.agent_indicator;
    if indicator.status == AgentStatus::Idle {
        return;
    }
    node.status = Some(indicator.status.wire_name().to_owned());
    node.status_pane = indicator.status_pane.map(|pane| pane.as_str().to_owned());
    node.status_blink = indicator.blink.then_some(true);
    node.status_color.clone_from(&indicator.color);
    node.status_shape = indicator.shape.map(|shape| shape.wire_name().to_owned());
}

pub(crate) fn epoch_seconds(time: SystemTime) -> f64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_secs_f64(),
        Err(before) => -before.duration().as_secs_f64(),
    }
}

/// The wall-clock time `seconds` after the Unix epoch, as [`epoch_seconds`] wrote it; `None` for a value no time can
/// hold (not finite, or out of range).
pub(crate) fn system_time(seconds: f64) -> Option<SystemTime> {
    let offset = std::time::Duration::try_from_secs_f64(seconds.abs()).ok()?;
    if seconds >= 0.0 {
        UNIX_EPOCH.checked_add(offset)
    } else {
        UNIX_EPOCH.checked_sub(offset)
    }
}

/// Whether every existing primary and split pane is backed by a live-session daemon (upstream's
/// `allPanesBackedByZmx`; the core's projection refines it with the dispositions, `Core::session_backed`).
pub(crate) fn all_panes_backed(session: &Session) -> bool {
    slot_backed(session, StatusPane::Left) && (!session.split.exists || slot_backed(session, StatusPane::Right))
}

/// Whether pane `role`'s terminal is a live-session client (upstream's `zmxBacking(for:)`): a pane of a split session
/// hidden since the launch has no terminal yet and answers with its sibling's, the launch it attaches with once shown.
fn slot_backed(session: &Session, role: StatusPane) -> bool {
    let live = |role: StatusPane| session.pane_slot(role).get().map(ngterm_host::Surface::is_live_session);
    live(role)
        .or_else(|| live(role.sibling().filter(|_| session.split.exists)?))
        .unwrap_or(false)
}

/// A pane's live font size, omitted while its terminal is not up.
fn font_size(slot: &SurfaceSlot) -> Option<f64> {
    slot.get()
        .filter(|surface| surface.is_realized())
        .and_then(ngterm_host::Surface::font_size)
}

/// The session's addressable surfaces in upstream's `TerminalZoomSurface` order (the primary pane, the split pane
/// while one exists, the scratch terminal while shown or alive, a program overlay and the pane overlays while open).
/// Only the primary and split panes report their live-session backing.
fn surface_nodes(session: &Session) -> Vec<ControlSurfaceNode> {
    ZoomSurface::ALL
        .into_iter()
        .filter(|kind| kind.is_available(session))
        .map(|kind| {
            let slot = kind.slot(session);
            let backed = kind
                .pane()
                .filter(|pane| matches!(pane, StatusPane::Left | StatusPane::Right));
            ControlSurfaceNode {
                id: SurfaceId {
                    session: session.id(),
                    surface: kind,
                }
                .control_id(),
                kind: kind.wire_name().to_owned(),
                active: kind.is_active(session),
                visible: kind.is_visible(session),
                backed_by_live: backed.map(|pane| slot_backed(session, pane)),
                lead: None,
                reconnect: None,
                pane_id: pane_token(slot),
            }
        })
        .collect()
}

/// The surface's `NGTERM_PANE_ID`, absent for an empty slot or a surface spawned without one.
pub(crate) fn pane_token(slot: &SurfaceSlot) -> Option<String> {
    slot.get()
        .map(ngterm_host::Surface::pane_token)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ngterm_host::fakes::FakeSurface;
    use serde_json::{Value, json};

    use super::*;
    use crate::model::{AgentIndicator, NewSession, SplitAxis, SplitLayout, StatusShape, WindowLibrary};

    fn app() -> AppIdentity {
        AppIdentity::from_recorded("0.1.0", Some("abc1234"))
    }

    fn tree_json(store: &AppStore, settings: &AppSettings, index_unsaved: bool) -> Value {
        let app = app();
        let context = TreeContext {
            app: &app,
            settings,
            index_unsaved,
            quick_visible: false,
        };
        serde_json::to_value(control_tree(store, &context)).unwrap()
    }

    fn id(uuid: Uuid) -> String {
        uuid_string(uuid)
    }

    #[test]
    fn a_fresh_window_projects_upstreams_defaults() {
        let mut library = WindowLibrary::seeded("/home/u");
        let store = library.active_store_mut().unwrap();
        let workspace = store.workspaces()[0].id();
        let session = store.selected_session_id().unwrap();
        // the seeding selection stamped it with the wall clock: pinned to compare
        let stamped = store.session_mut(session).unwrap();
        assert!(stamped.last_selected_at.is_some());
        stamped.last_selected_at = Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        let expected = json!({
            "workspaces": [{"id": id(workspace), "name": "workspace 1", "active": true, "sessions": [{
                "id": id(session), "name": "u", "cwd": "/home/u", "active": true, "split": false,
                "lastSelectedAt": 1_700_000_000.0,
                "backedByLive": false, "overlay": false, "scratch": false, "flagged": false,
                "surfaces": [{"id": format!("surface:{}:left", id(session)), "kind": "left", "active": true,
                              "visible": true, "backedByLive": false}],
                "realized": false,
            }]}],
            "attentionIncludesActive": false,
            "sidebarVisible": true, "sidebarMode": "tree", "sidebarFlaggedLayout": "flat", "linkOpenMode": "browser",
            "sidebarWidth": 220.0, "workspaceFilter": false, "quickVisible": false,
            "app": {"version": "0.1.0", "commit": "abc1234"},
        });
        assert_eq!(tree_json(store, &AppSettings::default(), false), expected);
    }

    #[test]
    fn each_window_projects_its_own_store() {
        let mut library = WindowLibrary::new("/home/u");
        let first = library.new_window(None);
        let second = library.new_window(None);
        let store = library.store_mut(second).unwrap();
        let workspace = store.add_workspace("build");
        let added = store.add_session(workspace, NewSession::at_cwd("/src")).unwrap();
        let first_tree = tree_json(library.store(first).unwrap(), &AppSettings::default(), false);
        let second_tree = tree_json(library.store(second).unwrap(), &AppSettings::default(), false);
        assert_eq!(first_tree["workspaces"].as_array().unwrap().len(), 1);
        let names: Vec<&str> = second_tree["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["workspace 1", "build"]);
        assert_eq!(second_tree["workspaces"][0]["active"], false);
        assert_eq!(second_tree["workspaces"][1]["active"], true);
        assert_eq!(second_tree["workspaces"][1]["sessions"][0]["id"], id(added));
        assert_eq!(second_tree["workspaces"][0]["sessions"][0]["active"], false);
    }

    /// A session with a shown top/bottom split, the split pane focused, both panes realized, its selection stamp
    /// pinned to 2s.
    fn split_session(store: &mut AppStore) -> Uuid {
        let workspace = store.workspaces()[0].id();
        let sid = store.add_session(workspace, NewSession::at_cwd("/repo")).unwrap();
        let session = store.session_mut(sid).unwrap();
        session.last_selected_at = Some(UNIX_EPOCH + Duration::from_secs(2));
        session.split = SplitLayout {
            shown: true,
            exists: true,
            focused: true,
            axis: SplitAxis::TopBottom,
            ratio: Some(0.3),
        };
        session.split_cwd = Some("/repo/sub".to_owned());
        session.surface.replace(Box::new(FakeSurface::new("TOKEN-L")));
        session
            .split_surface
            .replace(Box::new(FakeSurface::new("TOKEN-R").with_live_session(true)));
        sid
    }

    #[test]
    fn a_shown_split_reports_both_panes_with_the_focused_one_active() {
        let mut library = WindowLibrary::seeded("/home/u");
        let store = library.active_store_mut().unwrap();
        let sid = split_session(store);
        let tree = tree_json(store, &AppSettings::default(), false);
        let node = &tree["workspaces"][0]["sessions"][1];
        let surface = |kind: &str| format!("surface:{}:{kind}", id(sid));
        let expected = json!({
            "id": id(sid), "name": "sub", "cwd": "/repo", "splitCwd": "/repo/sub", "active": true, "split": true,
            "hasSplit": true, "backedByLive": false, "splitAxis": "horizontal", "splitRatio": 0.3,
            "splitFocused": true, "overlay": false, "scratch": false, "flagged": false,
            "fontSize": 13.0, "splitFontSize": 13.0, "lastSelectedAt": 2.0,
            "surfaces": [
                {"id": surface("left"), "kind": "left", "active": false, "visible": true, "backedByLive": false,
                 "paneID": "TOKEN-L"},
                {"id": surface("right"), "kind": "right", "active": true, "visible": true, "backedByLive": true,
                 "paneID": "TOKEN-R"},
            ],
            "realized": true,
        });
        assert_eq!(node, &expected);
    }

    #[test]
    fn a_hidden_split_keeps_its_fields_and_shows_only_the_focused_pane() {
        let mut library = WindowLibrary::seeded("/home/u");
        let store = library.active_store_mut().unwrap();
        let sid = split_session(store);
        let session = store.session_mut(sid).unwrap();
        session.split.shown = false;
        session.split.focused = false;
        session.scratch_active = true;
        let tree = tree_json(store, &AppSettings::default(), false);
        let node = &tree["workspaces"][0]["sessions"][1];
        assert_eq!(node["split"], false);
        assert_eq!(node["hasSplit"], true);
        assert_eq!(node["splitFocused"], false);
        assert_eq!(node["scratch"], true);
        let flags: Vec<(String, bool, bool)> = node["surfaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s["kind"].as_str().unwrap().to_owned(),
                    s["active"].as_bool().unwrap(),
                    s["visible"].as_bool().unwrap(),
                )
            })
            .collect();
        // the scratch terminal covers both panes
        assert_eq!(
            flags,
            [
                ("left".to_owned(), false, false),
                ("right".to_owned(), false, false),
                ("scratch".to_owned(), true, true)
            ]
        );
        session_mut_uncover(store, sid);
        let tree = tree_json(store, &AppSettings::default(), false);
        let surfaces = &tree["workspaces"][0]["sessions"][1]["surfaces"];
        assert_eq!(surfaces[0]["active"], true);
        assert_eq!(surfaces[0]["visible"], true);
        assert_eq!(surfaces[1]["visible"], false);
    }

    fn session_mut_uncover(store: &mut AppStore, sid: Uuid) {
        store.session_mut(sid).unwrap().scratch_active = false;
    }

    /// Upstream's `hiddenUnrealizedSplitTakesThePrimarysBacking` and `realizedUnbackedSplitStillClearsTheSessionBacking`.
    #[test]
    fn a_split_hidden_since_launch_takes_the_primarys_backing() {
        let mut library = WindowLibrary::seeded("/home/u");
        let store = library.active_store_mut().unwrap();
        let sid = split_session(store);
        let backing = |store: &AppStore| {
            let tree = tree_json(store, &AppSettings::default(), false);
            let node = &tree["workspaces"][0]["sessions"][1];
            (
                node["backedByLive"].clone(),
                node["surfaces"][1]["backedByLive"].clone(),
            )
        };
        for backed in [true, false] {
            let session = store.session_mut(sid).unwrap();
            (session.split.shown, session.split.focused) = (false, false);
            session.split_surface.teardown();
            session
                .surface
                .replace(Box::new(FakeSurface::new("TOKEN-L").with_live_session(backed)));
            assert_eq!(backing(store), (json!(backed), json!(backed)), "{backed}");
        }
        let session = store.session_mut(sid).unwrap();
        session
            .surface
            .replace(Box::new(FakeSurface::new("TOKEN-L").with_live_session(true)));
        session.split_surface.replace(Box::new(FakeSurface::new("TOKEN-R")));
        assert_eq!(backing(store), (json!(false), json!(false)));
    }

    #[test]
    fn a_promoted_survivor_drops_the_primary_surface() {
        let mut library = WindowLibrary::seeded("/home/u");
        let store = library.active_store_mut().unwrap();
        let sid = split_session(store);
        store.session_mut(sid).unwrap().surface.teardown();
        let tree = tree_json(store, &AppSettings::default(), false);
        let node = &tree["workspaces"][0]["sessions"][1];
        assert_eq!(node["surfaces"].as_array().unwrap().len(), 1);
        assert_eq!(node["surfaces"][0]["kind"], "right");
        assert_eq!(node["realized"], false);
    }

    #[test]
    fn status_metadata_and_sidebar_state_read_back() {
        let mut library = WindowLibrary::seeded("/home/u");
        let store = library.active_store_mut().unwrap();
        let workspace = store.workspaces()[0].id();
        let sid = store.selected_session_id().unwrap();
        store.set_focused_workspace(workspace);
        store.set_workspace_expanded(workspace, false);
        store.set_flag(true, sid);
        let session = store.session_mut(sid).unwrap();
        session.agent_indicator = AgentIndicator {
            status: AgentStatus::Blocked,
            blink: true,
            color: Some("#ff0000".to_owned()),
            shape: Some(StatusShape::Star),
            status_pane: Some(StatusPane::Left),
            ..AgentIndicator::default()
        };
        session.status_changed_at = Some(UNIX_EPOCH + Duration::from_millis(1_500));
        session.unseen_count = 3;
        session.osc_title = Some("vim".to_owned());
        session.context = Some("deploy".to_owned());
        session.launch.command = Some("make".to_owned());
        session.launch.wait = true;
        session.restore.command = Some(String::new());
        let settings = AppSettings {
            flagged_view_layout: Some("tree".to_owned()),
            link_open_mode: Some("overlay".to_owned()),
            auto_follow_attention: Some("s30".to_owned()),
            ..AppSettings::default()
        };
        let tree = tree_json(store, &settings, true);
        assert_eq!(tree["sidebarFlaggedLayout"], "tree");
        assert_eq!(tree["linkOpenMode"], "overlay");
        assert_eq!(tree["autoFollowMs"], 30_000);
        assert_eq!(tree["indexUnsaved"], true);
        let workspace_node = &tree["workspaces"][0];
        assert_eq!(workspace_node["focused"], true);
        assert_eq!(workspace_node["collapsed"], true);
        let node = &workspace_node["sessions"][0];
        for (key, value) in [
            // the title names no session by default, so `name` is the cwd basename beside the raw title
            ("name", json!("u")),
            ("title", json!("vim")),
            ("flagged", json!(true)),
            ("status", json!("blocked")),
            ("statusPane", json!("left")),
            ("statusBlink", json!(true)),
            ("statusColor", json!("#ff0000")),
            ("statusShape", json!("star")),
            ("statusChangedAt", json!(1.5)),
            ("unseen", json!(3)),
            ("context", json!("deploy")),
            ("commandWait", json!(true)),
            ("restoreCommand", json!("")),
        ] {
            assert_eq!(node[key], value, "{key}");
        }
    }

    #[test]
    fn an_idle_status_reports_only_when_it_changed() {
        let mut library = WindowLibrary::seeded("/home/u");
        let store = library.active_store_mut().unwrap();
        let sid = store.selected_session_id().unwrap();
        let session = store.session_mut(sid).unwrap();
        session.agent_indicator = AgentIndicator {
            blink: true,
            color: Some("#00ff00".to_owned()),
            shape: Some(StatusShape::Diamond),
            ..AgentIndicator::default()
        };
        session.status_changed_at = Some(UNIX_EPOCH + Duration::from_secs(2));
        let tree = tree_json(store, &AppSettings::default(), false);
        let node = tree["workspaces"][0]["sessions"][0].as_object().unwrap();
        for key in ["status", "statusPane", "statusBlink", "statusColor", "statusShape"] {
            assert!(!node.contains_key(key), "{key}");
        }
        // `statusChangedAt` is reported whenever it exists, idle included (`control-api.md`)
        assert_eq!(node["statusChangedAt"], json!(2.0));
    }

    /// Upstream's tree projection pins of `lastSelectedAt`: each session's own stamp as epoch seconds, and no key for
    /// a session with none.
    #[test]
    fn last_selected_at_projects_each_sessions_stamp() {
        let mut store = AppStore::new();
        let work = store.add_workspace("work");
        let a = store.add_session(work, NewSession::at_cwd("/repo/a")).unwrap();
        let b = store.add_session(work, NewSession::at_cwd("/repo/b")).unwrap();
        let background = NewSession {
            select: false,
            ..NewSession::at_cwd("/repo/c")
        };
        let never = store.add_session(work, background).unwrap();
        store.session_mut(a).unwrap().last_selected_at = system_time(1_600_000_000.0);
        store.session_mut(b).unwrap().last_selected_at = system_time(1_700_000_100.0);
        store.session_mut(never).unwrap().last_selected_at = None;
        let tree = tree_json(&store, &AppSettings::default(), false);
        let nodes = &tree["workspaces"][0]["sessions"];
        assert_eq!(nodes[0]["lastSelectedAt"], json!(1_600_000_000.0));
        assert_eq!(nodes[1]["lastSelectedAt"], json!(1_700_000_100.0));
        assert!(!nodes[2].as_object().unwrap().contains_key("lastSelectedAt"));
    }
}
