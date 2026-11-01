//! The glyph each Settings ▸ Interface element's control shows at rest (upstream's `InterfaceElement.symbolName`,
//! #810), shared by that control and its Settings toggle so the two cannot drift: the title bar's buttons
//! ([`crate::titlebar`]), the sidebar footer's and the workspace row's ([`crate::sidebar`]), and the Interface tab's
//! rows ([`crate::settings`]), each leading with its control's glyph in an 18px slot, left empty for the text-only
//! elements ([`InterfaceElement::text_only`]).
//!
//! A glyph comes from the drawing its control already uses: Lucide's catalog for the title bar's plain buttons (served
//! by [`crate::titlebar::Icons`]), the title bar's traced pane glyphs for the scratch and split toggles, and the
//! sidebar's masks ([`crate::row_icon`]) for the footer and the row's "+". A control whose glyph follows its state
//! (scratch shown, split panes, overlays hidden, the focus filter or the flagged view on) shows the one here in its
//! neutral state.
//!
//! The dashboard is Lucide's `grid-2x2`, a frame split in four (upstream's `rectangle.split.2x2`), not `layout-grid`'s
//! four squares, which read as the focus filter's `square.grid.2x2`.

use gpui_kit::assets::IconName;
use gpui_kit::{AnyElement, Hsla, IntoElement as _, Pixels};
use ngterm_core::persist::settings::InterfaceElement;
use ngterm_core::titlebar::SplitGlyph;

use crate::row_icon::{self, Glyph};
use crate::titlebar::PaneGlyph;

/// The Settings rows' glyph slot, wide enough for every glyph so the labels align (upstream's `.frame(width: 18)`).
pub const SLOT: f32 = 18.;

/// Where an element's glyph is drawn from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementGlyph {
    /// A Lucide glyph (the title bar's plain buttons and the remote host).
    Icon(IconName),
    /// A traced pane glyph (the title bar's scratch and split toggles).
    Pane(PaneGlyph),
    /// A sidebar mask (the footer's controls and the workspace row's "+"), always in its 16x16 slot.
    Row(Glyph),
}

pub const SIDEBAR_TOGGLE: IconName = IconName::PanelLeft;
pub const REMOTE_HOST: IconName = IconName::Cloud;
pub const RECENT_SESSIONS: IconName = IconName::RotateCcwClock;
pub const SCRATCH: PaneGlyph = PaneGlyph::Scratch(false);
pub const SPLIT: PaneGlyph = PaneGlyph::Split(SplitGlyph::None);
pub const OVERLAY_TOGGLE: IconName = IconName::Copy;
pub const DASHBOARD: IconName = IconName::Grid2x2;
pub const QUICK_TERMINAL: IconName = IconName::SquareTerminal;
pub const CUSTOM_COMMANDS: IconName = IconName::CircleEllipsis;
pub const NEW_WORKSPACE: Glyph = Glyph::NewWorkspace;
pub const NEW_SESSION: Glyph = Glyph::AddSession;
pub const FLAGGED_VIEW: Glyph = Glyph::Flag { filled: false };
pub const FOCUS_FILTER: Glyph = Glyph::Grid { filled: false };
pub const WORKSPACE_ADD_SESSION: Glyph = Glyph::Plus;

/// `element`'s glyph at rest; `None` for a text-only element.
#[must_use]
pub fn of(element: InterfaceElement) -> Option<ElementGlyph> {
    use ElementGlyph::{Icon, Pane, Row};
    Some(match element {
        InterfaceElement::WorkspaceName
        | InterfaceElement::SessionName
        | InterfaceElement::WindowName
        | InterfaceElement::SessionContext => return None,
        InterfaceElement::SidebarToggle => Icon(SIDEBAR_TOGGLE),
        InterfaceElement::RemoteHost => Icon(REMOTE_HOST),
        InterfaceElement::RecentSessions => Icon(RECENT_SESSIONS),
        InterfaceElement::Scratch => Pane(SCRATCH),
        InterfaceElement::Split => Pane(SPLIT),
        InterfaceElement::OverlayToggle => Icon(OVERLAY_TOGGLE),
        InterfaceElement::Dashboard => Icon(DASHBOARD),
        InterfaceElement::QuickTerminal => Icon(QUICK_TERMINAL),
        InterfaceElement::CustomCommands => Icon(CUSTOM_COMMANDS),
        InterfaceElement::NewWorkspace => Row(NEW_WORKSPACE),
        InterfaceElement::NewSession => Row(NEW_SESSION),
        InterfaceElement::FlaggedView => Row(FLAGGED_VIEW),
        InterfaceElement::FocusFilter => Row(FOCUS_FILTER),
        InterfaceElement::WorkspaceAddSession => Row(WORKSPACE_ADD_SESSION),
    })
}

impl ElementGlyph {
    /// The glyph in `ink` at `size` (a sidebar mask keeps its 16x16 slot).
    #[must_use]
    pub fn draw(self, ink: Hsla, size: Pixels) -> AnyElement {
        match self {
            Self::Icon(name) => crate::titlebar::icon(name, ink, size),
            Self::Pane(glyph) => crate::titlebar::pane_glyph(glyph, ink, size).into_any_element(),
            Self::Row(glyph) => row_icon::icon(glyph, ink).into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Upstream's `everyControlElementHasADistinctSymbolAndTextElementsHaveNone`, the glyph half (the text-only set is
    /// `ngterm-core`'s test).
    #[test]
    fn every_control_element_has_a_distinct_glyph_and_text_elements_have_none() {
        let mut glyphs = Vec::new();
        for element in InterfaceElement::ALL {
            let glyph = of(*element);
            assert_eq!(glyph.is_none(), element.text_only(), "{element:?}");
            if let Some(glyph) = glyph {
                assert!(!glyphs.contains(&glyph), "{element:?} repeats {glyph:?}");
                glyphs.push(glyph);
            }
        }
    }

    /// Upstream's `testEveryInterfaceElementSymbolResolves`: a Lucide glyph is served by the app's assets, a traced one
    /// draws something.
    #[test]
    fn every_interface_element_glyph_resolves() {
        for element in InterfaceElement::ALL {
            match of(*element) {
                None => {}
                Some(ElementGlyph::Icon(name)) => {
                    assert!(crate::titlebar::serves(&name.path()), "{element:?}: {}", name.path());
                }
                Some(ElementGlyph::Pane(glyph)) => {
                    assert!(!crate::titlebar::glyph_strokes(glyph).is_empty(), "{element:?}");
                }
                Some(ElementGlyph::Row(glyph)) => {
                    let mask = row_icon::rasterize(glyph, 32);
                    assert!(mask.iter().any(|&coverage| coverage > 0), "{element:?}");
                }
            }
        }
    }

    /// The dashboard and the focus filter, both a 2x2 grid upstream (`rectangle.split.2x2`, `square.grid.2x2`), are
    /// drawn from different shapes: a frame split in four against four separate squares.
    #[test]
    fn the_dashboard_and_focus_filter_glyphs_read_as_distinct() {
        assert_eq!(
            of(InterfaceElement::Dashboard),
            Some(ElementGlyph::Icon(IconName::Grid2x2))
        );
        assert_eq!(
            of(InterfaceElement::FocusFilter),
            Some(ElementGlyph::Row(Glyph::Grid { filled: false }))
        );
    }
}
