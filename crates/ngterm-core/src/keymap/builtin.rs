//! The rebindable built-in actions (upstream's `BuiltinAction.swift`): each case's `keymap.conf` name, in
//! upstream's declaration order, which `keymap.list` and the starter file keep.

use strum::{EnumIter, EnumString, IntoStaticStr};

/// A rebindable, menu-backed built-in action, named as the token written after `map` in `keymap.conf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, EnumIter, EnumString, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum BuiltinAction {
    NewWindow,
    RenameWindow,
    DeleteWindow,
    PreviousWindow,
    NextWindow,
    NewWorkspace,
    RenameWorkspace,
    DeleteWorkspace,
    NewSession,
    OpenDirectory,
    RenameSession,
    DuplicateSession,
    NewCompanion,
    CloseSession,
    ReopenRecent,
    UndoClose,
    ClearStatus,
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    ToggleSplit,
    ToggleHorizontalSplit,
    ToggleScratch,
    ToggleTerminalZoom,
    ToggleOverlay,
    ToggleSearch,
    ToggleSidebar,
    SelectTheme,
    ToggleFullscreen,
    ToggleFlaggedView,
    ToggleFlag,
    FocusWorkspace,
    ToggleWorkspaceFilter,
    PreviousWorkspace,
    NextWorkspace,
    ToggleWorkspaceCollapse,
    FocusLeftPane,
    FocusRightPane,
    PreviousSession,
    NextSession,
    PreviousAttentionSession,
    NextAttentionSession,
    FirstSession,
    LastSession,
    QuickTerminal,
    SessionPalette,
    CommandPalette,
    CustomCommandPalette,
    ShowAttention,
    Dashboard,
    AttachRemote,
    EditFlaggedSessions,
    EditFocusedWorkspaces,
}

impl BuiltinAction {
    /// The `keymap.conf` name.
    #[must_use]
    pub fn name(self) -> &'static str {
        self.into()
    }

    /// Every action, in upstream's declaration order.
    pub fn all() -> impl Iterator<Item = Self> {
        <Self as strum::IntoEnumIterator>::iter()
    }

    /// The action a `keymap.conf` name spells, exactly (no case folding or trimming).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        name.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifty_three_actions_with_upstreams_names() {
        let names: Vec<&str> = BuiltinAction::all().map(BuiltinAction::name).collect();
        assert_eq!(names.len(), 53);
        assert_eq!(names[0], "new_window");
        assert_eq!(names[11..13], ["duplicate_session", "new_companion"]);
        assert_eq!(names[50], "attach_remote");
        assert_eq!(names[51..], ["edit_flagged_sessions", "edit_focused_workspaces"]);
        assert_eq!(BuiltinAction::ToggleOverlay.name(), "toggle_overlay");
        assert_eq!(
            names[24], "toggle_overlay",
            "after toggle_terminal_zoom, as upstream declares it"
        );
        assert!(names.contains(&"toggle_horizontal_split"));
        assert!(names.contains(&"previous_attention_session"));
        for name in names {
            assert_eq!(BuiltinAction::from_name(name).map(BuiltinAction::name), Some(name));
        }
        assert_eq!(BuiltinAction::from_name("Toggle_split"), None);
    }
}
