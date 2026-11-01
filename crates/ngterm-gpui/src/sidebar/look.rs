//! What the sidebar is painted with: the window chrome's colors ([`crate::chrome`], the one source the window body,
//! the title bar and the sidebar share: the GTK build's `chrome.rs` stylesheet for the sidebar, upstream's
//! `sidebarTintWash`, `SidebarRowView` and `SidebarCellView.setColors`) and the rows' metrics (upstream's
//! `WorkspaceSidebar+RowRendering.swift` `makeCell`, `SidebarRowViews.swift` `BadgeView`, `WindowContentView.swift`
//! `bottomBar`).

use gpui_kit::Hsla;
use ngterm_core::chrome::{ChromeColors, Rgb};
use ngterm_core::model::{SidebarTint, TintColor, TintPart};
use ngterm_core::persist::AppSettings;

pub use crate::chrome::color;

/// The rows' text size when the setting leaves it unset (upstream's `sidebarFontSize` default, the system font size).
pub const DEFAULT_FONT_SIZE: f32 = 13.0;
/// The `sidebarFontSize` setting's range.
const FONT_SIZE_RANGE: (f64, f64) = (9.0, 20.0);
/// A row's height over its text size: 26 at the default 13.
const ROW_PADDING: f32 = 13.0;
/// The selection pill's inset from the row's edges and its corner radius (upstream's `SidebarRowView.drawBackground`).
pub const PILL_INSET_X: f32 = 8.0;
pub const PILL_INSET_Y: f32 = 1.5;
pub const PILL_RADIUS: f32 = 7.0;
/// The content's inset from the row's edges: inside the pill, the icon two points in.
pub const CONTENT_INSET: f32 = PILL_INSET_X + 2.0;
/// The gap between the leading icon and the name, and between the name and what trails it.
pub const GAP: f32 = 6.0;
/// The disclosure's slot before a workspace row's icon, and the indent of a nested session row: its icon sits under
/// its workspace's.
pub const DISCLOSURE: f32 = 16.0;
pub const NESTED_INDENT: f32 = DISCLOSURE + 4.0;
/// The unseen badge: the capsule's height (its minimum width), the room the count's text gets, the gap before it, and
/// its text size (upstream's `BadgeView`, `NSFont.smallSystemFontSize` bold).
pub const BADGE_HEIGHT: f32 = 16.0;
pub const BADGE_TEXT_ROOM: f32 = 9.0;
pub const BADGE_GAP: f32 = 4.0;
pub const BADGE_FONT_SIZE: f32 = 11.0;
/// macOS `systemRed`, the badge's capsule (the conventional unread color, readable on the selected row too).
pub const BADGE_RED: Rgb = Rgb::new(0xFF, 0x3B, 0x30);
/// The icons' opacity over the row's text color, unselected and selected (upstream's `setColors`).
pub const ICON_ALPHA: f64 = 0.6;
pub const SELECTED_ICON_ALPHA: f64 = 0.85;
/// The footer: its controls' frame, the gap between them, and its padding (upstream's `bottomBar`).
pub const FOOTER_BUTTON: (f32, f32) = (24.0, 22.0);
pub const FOOTER_GAP: f32 = 2.0;
pub const FOOTER_PADDING: (f32, f32) = (6.0, 4.0);
/// A footer toggle with nothing to toggle is muted by hand (upstream's 0.35).
pub const DISABLED_ALPHA: f32 = 0.35;
/// The divider's grab handle: wider than the 1-point line it centers on (upstream's `sidebarDivider`).
pub const DIVIDER_HANDLE: f32 = 12.0;

/// The sidebar's colors and text size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub colors: ChromeColors,
    pub font_size: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            colors: crate::chrome::default_colors(),
            font_size: DEFAULT_FONT_SIZE,
        }
    }
}

impl Look {
    /// The look of the chrome's `colors` (the window's, [`crate::chrome::current`]) under `settings`: the rows' text
    /// size.
    #[must_use]
    pub fn of(colors: ChromeColors, settings: &AppSettings) -> Self {
        Self {
            colors,
            font_size: font_size(settings.sidebar_font_size),
        }
    }

    /// A row's height.
    #[must_use]
    pub fn row_height(&self) -> f32 {
        self.font_size + ROW_PADDING
    }

    /// The divider's line: the chrome text at low opacity.
    #[must_use]
    pub fn separator(&self) -> Hsla {
        crate::chrome::separator(&self.colors)
    }

    /// The text of a row, selected or not.
    #[must_use]
    pub fn text(&self, selected: bool) -> Hsla {
        if selected {
            crate::chrome::selected_row(&self.colors, true).1
        } else {
            color(self.colors.foreground, 1.0)
        }
    }

    /// The icons of a row, selected or not: its text color, dimmed.
    #[must_use]
    pub fn icon(&self, selected: bool) -> Hsla {
        let alpha = if selected { SELECTED_ICON_ALPHA } else { ICON_ALPHA };
        let base = if selected {
            self.colors.selection_foreground
        } else {
            self.colors.foreground
        };
        color(base, alpha)
    }

    /// The selected rows' pill: the selection background, dimmed while the window is not the active one, else
    /// upstream's soft wash of the theme foreground.
    #[must_use]
    pub fn pill(&self, key_window: bool) -> Hsla {
        crate::chrome::selected_row(&self.colors, key_window).0
    }

    /// Whether the chrome reads dark, the side a tint's color is taken from (the gpui-kit theme's side).
    #[must_use]
    pub fn dark(&self) -> bool {
        crate::chrome::kit::mode(&self.colors).is_dark()
    }

    /// A row's name and icon colors, selected or not: a `tint` overrides the parts it colors, at full strength and
    /// whatever the selection (upstream's `SidebarCellView.setColors`).
    #[must_use]
    pub fn row_inks(&self, tint: Option<SidebarTint>, selected: bool) -> (Hsla, Hsla) {
        let tinted = |part| {
            tint.filter(|tint| tint.shows(part))
                .map(|tint| self::tint(tint.color(), self.dark()))
        };
        (
            tinted(TintPart::Name).unwrap_or_else(|| self.text(selected)),
            tinted(TintPart::Icon).unwrap_or_else(|| self.icon(selected)),
        )
    }
}

/// A tint color: the macOS system color of the same name (`NSColor.systemRed` and kin), its dark-appearance variant
/// on a dark chrome, as `AppKit`'s dynamic colors follow the appearance.
#[must_use]
pub fn tint_rgb(tint: TintColor, dark: bool) -> Rgb {
    let (light, dark_variant) = match tint {
        TintColor::Red => (Rgb::new(0xFF, 0x3B, 0x30), Rgb::new(0xFF, 0x45, 0x3A)),
        TintColor::Orange => (Rgb::new(0xFF, 0x95, 0x00), Rgb::new(0xFF, 0x9F, 0x0A)),
        TintColor::Yellow => (Rgb::new(0xFF, 0xCC, 0x00), Rgb::new(0xFF, 0xD6, 0x0A)),
        TintColor::Green => (Rgb::new(0x28, 0xCD, 0x41), Rgb::new(0x32, 0xD7, 0x4B)),
        TintColor::Blue => (Rgb::new(0x00, 0x7A, 0xFF), Rgb::new(0x0A, 0x84, 0xFF)),
        TintColor::Purple => (Rgb::new(0xAF, 0x52, 0xDE), Rgb::new(0xBF, 0x5A, 0xF2)),
        TintColor::Gray => (Rgb::new(0x8E, 0x8E, 0x93), Rgb::new(0x98, 0x98, 0x9D)),
    };
    if dark { dark_variant } else { light }
}

/// [`tint_rgb`] as a fill.
#[must_use]
pub fn tint(tint: TintColor, dark: bool) -> Hsla {
    color(tint_rgb(tint, dark), 1.0)
}

/// The rows' text size from the `sidebarFontSize` setting, clamped to 9...20; 13 when unset.
#[must_use]
pub fn font_size(setting: Option<f64>) -> f32 {
    let size = setting.map_or(f64::from(DEFAULT_FONT_SIZE), |size| {
        size.clamp(FONT_SIZE_RANGE.0, FONT_SIZE_RANGE.1)
    });
    #[expect(clippy::cast_possible_truncation, reason = "clamped to 9...20")]
    let size = size as f32;
    size
}
