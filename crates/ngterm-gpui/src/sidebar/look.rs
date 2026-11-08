//! What the sidebar is painted with: the window chrome's colors ([`crate::chrome`], the one source the window body,
//! the title bar and the sidebar share: the GTK build's `chrome.rs` stylesheet for the sidebar, upstream's
//! `sidebarTintWash`, `SidebarRowView` and `SidebarCellView.setColors`) and the rows' metrics (upstream's
//! `WorkspaceSidebar+RowRendering.swift` `makeCell`, `SidebarRowViews.swift` `BadgeView`, `WindowContentView.swift`
//! `bottomBar`), with the compact density's trims (upstream's `SidebarDensity`).

use gpui_kit::Hsla;
use ngterm_core::chrome::{ChromeColors, Rgb};
use ngterm_core::model::{SidebarTint, TintColor, TintPart, TintValue};
use ngterm_core::persist::AppSettings;
use ngterm_core::persist::settings::{InterfaceElement, SidebarDensity};

pub use crate::chrome::color;

/// The rows' text size when the setting leaves it unset (upstream's `sidebarFontSize` default, the system font size).
pub const DEFAULT_FONT_SIZE: f32 = 13.0;
/// The `sidebarFontSize` setting's range.
const FONT_SIZE_RANGE: (f64, f64) = (9.0, 20.0);
/// A row's height over its text size: 26 at the default 13. This build's rows run 2 points shorter than upstream's (15);
/// the compact density takes upstream's trims off it.
const ROW_PADDING: f32 = 13.0;
/// The selection pill's inset from the row's edges and its corner radius (upstream's `SidebarRowView.drawBackground`).
/// The vertical inset is the default density's ([`Metrics::pill_inset_y`]).
pub const PILL_INSET_X: f32 = 8.0;
const PILL_INSET_Y: f32 = 1.5;
pub const PILL_RADIUS: f32 = 7.0;
/// The content's inset from the row's edges: inside the pill, the icon two points in.
pub const CONTENT_INSET: f32 = PILL_INSET_X + 2.0;
/// The gap between the leading icon and the name, and between the name and what trails it.
pub const GAP: f32 = 6.0;
/// The disclosure's slot before a workspace row's icon, and the indent of a nested session row: its icon sits under
/// its workspace's.
pub const DISCLOSURE: f32 = 16.0;
const NESTED_INDENT: f32 = DISCLOSURE + 4.0;
/// How far an info line lifts into the row above it, and the padding under it, at the default density.
const INFO_SPACING: f32 = 6.0;
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
/// The divider's grab handle: wider than the 1-point line it centers on (upstream's `sidebarDivider`).
pub const DIVIDER_HANDLE: f32 = 12.0;

/// The row geometry a [`SidebarDensity`] sets (upstream's `SidebarDensity` metrics). Compact applies upstream's deltas
/// to this build's default geometry: 5 points off the row padding, 4 off the nesting indent, 2 off the info line's lift
/// and bottom padding, the pill's vertical inset down to 0.5, and the name lifted 1 point so lowercase text sits
/// centered in the tighter pill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// The vertical padding a row adds to its text size.
    pub row_padding: f32,
    /// The step per nesting level, a workspace row's disclosure taking its first.
    pub nested_indent: f32,
    /// How far an info line lifts into the row above it, and the padding under it.
    pub info_spacing: f32,
    /// How far the name sits above the row's center.
    pub name_lift: f32,
    /// The selection pill's (and the drop frame's) inset from the row's top and bottom edges.
    pub pill_inset_y: f32,
}

impl Metrics {
    /// The geometry of `density`.
    #[must_use]
    pub fn of(density: SidebarDensity) -> Self {
        match density {
            SidebarDensity::Default => Self {
                row_padding: ROW_PADDING,
                nested_indent: NESTED_INDENT,
                info_spacing: INFO_SPACING,
                name_lift: 0.0,
                pill_inset_y: PILL_INSET_Y,
            },
            SidebarDensity::Compact => Self {
                row_padding: ROW_PADDING - 5.0,
                nested_indent: NESTED_INDENT - 4.0,
                info_spacing: INFO_SPACING - 2.0,
                name_lift: 1.0,
                pill_inset_y: 0.5,
            },
        }
    }
}

/// The sidebar's colors, text size and row spacing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub colors: ChromeColors,
    pub font_size: f32,
    pub density: SidebarDensity,
    /// Workspace rows draw their disclosure chevron and reserve its strip (the `workspaceChevrons` interface element).
    pub chevrons: bool,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            colors: crate::chrome::default_colors(),
            font_size: DEFAULT_FONT_SIZE,
            density: SidebarDensity::Default,
            chevrons: true,
        }
    }
}

impl Look {
    /// The look of the chrome's `colors` (the window's, [`crate::chrome::current`]) under `settings`: the rows' text
    /// size, density and whether workspace rows show their chevron.
    #[must_use]
    pub fn of(colors: ChromeColors, settings: &AppSettings) -> Self {
        Self {
            colors,
            font_size: font_size(settings.sidebar_font_size),
            density: settings.effective_sidebar_density(),
            chevrons: !settings.is_interface_element_hidden(InterfaceElement::WorkspaceChevrons),
        }
    }

    /// The row geometry of the look's density.
    #[must_use]
    pub fn metrics(&self) -> Metrics {
        Metrics::of(self.density)
    }

    /// A row's height (an info row's title band).
    #[must_use]
    pub fn row_height(&self) -> f32 {
        self.font_size + self.metrics().row_padding
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
                .map(|tint| tint_value(tint.color(), self.dark()))
        };
        (
            tinted(TintPart::Name).unwrap_or_else(|| self.text(selected)),
            tinted(TintPart::Icon).unwrap_or_else(|| self.icon(selected)),
        )
    }
}

/// A tint color: the macOS system color of the same name (`NSColor.systemRed` and kin), its dark-appearance variant
/// on a dark chrome, as `AppKit`'s dynamic colors follow the appearance. The five upstream added in v0.41.0 (teal to
/// mint) are their sRGB values read from macOS 27.2 in the aqua and dark aqua appearances.
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
        TintColor::Teal => (Rgb::new(0x00, 0xC3, 0xD0), Rgb::new(0x00, 0xD2, 0xE0)),
        TintColor::Pink => (Rgb::new(0xFF, 0x2D, 0x55), Rgb::new(0xFF, 0x37, 0x5F)),
        TintColor::Brown => (Rgb::new(0xAC, 0x7F, 0x5E), Rgb::new(0xB7, 0x8A, 0x66)),
        TintColor::Indigo => (Rgb::new(0x61, 0x55, 0xF5), Rgb::new(0x6D, 0x7C, 0xFF)),
        TintColor::Mint => (Rgb::new(0x00, 0xC8, 0xB3), Rgb::new(0x00, 0xDA, 0xC3)),
    };
    if dark { dark_variant } else { light }
}

/// [`tint_rgb`] as a fill.
#[must_use]
pub fn tint(tint: TintColor, dark: bool) -> Hsla {
    color(tint_rgb(tint, dark), 1.0)
}

/// A row tint's color (upstream's `NSColor.tint(_: TintValue)`): a name as [`tint_rgb`], a hex as its fixed sRGB on
/// either side.
#[must_use]
pub fn tint_value_rgb(value: TintValue, dark: bool) -> Rgb {
    match value {
        TintValue::Named(name) => tint_rgb(name, dark),
        TintValue::Hex([red, green, blue]) => Rgb::new(red, green, blue),
    }
}

/// [`tint_value_rgb`] as a fill.
#[must_use]
pub fn tint_value(value: TintValue, dark: bool) -> Hsla {
    color(tint_value_rgb(value, dark), 1.0)
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
