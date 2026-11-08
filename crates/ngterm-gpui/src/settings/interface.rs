//! Interface: which title-bar and sidebar chrome elements show, two toggles per row as upstream lays them out, the
//! session info picker and the sidebar in inactive windows sharing the Sidebar section's last row, and the quick
//! terminal's size (upstream's `InterfaceSettingsView`). Each element's toggle leads with its control's glyph at rest
//! ([`crate::interface_glyph`]), the text-only elements' slot left empty and listed first
//! ([`InterfaceElement::settings_order`], #810).

use ngterm_core::persist::settings::{InterfaceElement, SessionInfoDisplay};
use ngterm_core::quick_metrics::SIZE_PERCENT_CHOICES;
use ngterm_core::settings_model::{InterfaceChange, SettingsChange};

use super::Snapshot;
use super::form::{Control, Item, Picker, Section, picker, toggle};

fn change(change: InterfaceChange) -> SettingsChange {
    SettingsChange::Interface(change)
}

/// The page's sections.
#[must_use]
pub fn sections(snapshot: &Snapshot) -> Vec<Section> {
    vec![
        Section::new("Title Bar", elements(snapshot, true)).two_columns(),
        // the info picker and the multi-window toggle share the Sidebar section's last row, as upstream's, which keeps
        // the tab short enough to show the quick terminal's Size without scrolling: the grid's eight toggles leave no
        // empty half, so the two close the grid as its last pair (#909).
        Section::new("Sidebar", sidebar_items(snapshot)).two_columns(),
        Section::new("Quick Terminal", vec![quick_terminal_size(snapshot)]),
    ]
}

/// The Sidebar section's grid: its elements' toggles, then the info picker and the active-window toggle as the last
/// pair, the toggle's glyph slot left empty so its label lines up with the glyph rows'.
fn sidebar_items(snapshot: &Snapshot) -> Vec<Item> {
    let settings = &snapshot.settings;
    let mut items = elements(snapshot, false);
    debug_assert!(
        items.len().is_multiple_of(2),
        "the picker and the toggle pair up only after an even grid"
    );
    items.push(
        picker(
            "settings-session-info-display",
            "Session info",
            &SessionInfoDisplay::ALL
                .iter()
                .map(|display| (display.display_name(), *display))
                .collect::<Vec<_>>(),
            &settings.effective_session_info_display(),
            |display| change(InterfaceChange::SessionInfoDisplay(display)),
        )
        .leading(None),
    );
    items.push(
        toggle(
            "settings-auto-hide-inactive-sidebars",
            "Active window only",
            settings.auto_hide_sidebar_inactive_windows.unwrap_or(false),
            |on| change(InterfaceChange::AutoHideSidebarInactiveWindows(on)),
        )
        .leading(None),
    );
    items
}

/// One toggle per element of the title bar (or the sidebar): on is shown, so hiding a default-shown element is the edit
/// that writes.
fn elements(snapshot: &Snapshot, title_bar: bool) -> Vec<Item> {
    InterfaceElement::settings_order(title_bar)
        .into_iter()
        .map(|element| {
            toggle(
                &format!("settings-interface-{}", element.raw()),
                element.display_name(),
                !snapshot.settings.is_interface_element_hidden(element),
                move |visible| change(InterfaceChange::ElementVisible { element, visible }),
            )
            .leading(crate::interface_glyph::of(element))
        })
        .collect()
}

/// The quick terminal's share of the screen; Default is the built-in size, not a percentage.
fn quick_terminal_size(snapshot: &Snapshot) -> Item {
    let sizes: Vec<(String, Option<i64>)> = std::iter::once(("Default".to_owned(), None))
        .chain(
            SIZE_PERCENT_CHOICES
                .iter()
                .map(|percent| (format!("{percent}% of screen"), Some(*percent))),
        )
        .collect();
    Item::new(
        "settings-quick-terminal-size",
        "Size",
        Control::Picker(Picker::new(
            sizes,
            &snapshot.settings.effective_quick_terminal_size_percent(),
            |percent| change(InterfaceChange::QuickTerminalSizePercent(percent)),
        )),
    )
}
