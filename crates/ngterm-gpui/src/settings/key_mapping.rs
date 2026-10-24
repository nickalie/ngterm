//! Key Mapping: the keyboard layout held while a terminal has the keyboard (`crate::input_source`), the config
//! directory holding `keymap.conf` and `hooks.conf` (Choose…, and Use Default while one is set), and each file's
//! diagnostics with its Reload (upstream's `KeyMappingSettingsView`).

use ngterm_core::settings_model::SettingsChange;

use super::Snapshot;
use super::form::{Action, Control, Item, Picker, Run, Section};
use crate::folder_chooser::FolderRequest;

/// The page's sections.
#[must_use]
pub fn sections(snapshot: &Snapshot) -> Vec<Section> {
    vec![
        Section::new("Keyboard Layout", keyboard_layout(snapshot)),
        Section::new("Config Directory", directory(snapshot)),
        Section::new(
            "Diagnostics",
            diagnostics("settings-keymap", &snapshot.keymap_diagnostics, |core| {
                core.reload_keymap();
            }),
        ),
        Section::new(
            "Hooks",
            diagnostics("settings-hooks", &snapshot.hooks_diagnostics, |core| {
                core.reload_hooks();
            }),
        ),
    ]
}

/// "Lock in terminals": Off first, then the layouts the system can select, then a saved layout it no longer can,
/// as "<id> (unavailable)", so the picker still shows the stored choice; disabled with the reason where this desktop
/// has no way to switch layouts.
fn keyboard_layout(snapshot: &Snapshot) -> Vec<Item> {
    let layouts = &snapshot.layouts;
    let saved = snapshot.settings.terminal_input_source.clone();
    let mut options: Vec<(String, Option<String>)> = vec![("Off".to_owned(), None)];
    options.extend(
        layouts
            .selectable
            .iter()
            .map(|layout| (layout.name.clone(), Some(layout.id.clone()))),
    );
    if let Some(id) = saved
        .as_deref()
        .filter(|id| !layouts.selectable.iter().any(|layout| layout.id == *id))
    {
        options.push((format!("{id} (unavailable)"), Some(id.to_owned())));
    }
    let picker = Picker::new(options, &saved, SettingsChange::TerminalInputSource);
    let mut items = vec![
        Item::new(
            "settings-terminal-input-source",
            "Lock in terminals",
            Control::Picker(picker),
        )
        .enabled(layouts.unavailable.is_none()),
    ];
    if let Some(reason) = &layouts.unavailable {
        items.push(Item::hint("settings-terminal-input-source-unavailable", reason.clone()));
    }
    items
}

/// The resolved directory, Choose… and, while one is set, Use Default; a change reloads both files.
fn directory(snapshot: &Snapshot) -> Vec<Item> {
    let mut buttons = vec![Action {
        id: "settings-keymap-choose",
        label: "Choose…",
        run: Run::Choose {
            request: FolderRequest {
                title: "Choose a directory for keymap.conf",
                accept: "Choose",
                start: snapshot.chooser_start(Some(&snapshot.config_dir)),
            },
            change: |path| SettingsChange::ConfigDirectory(Some(path)),
        },
    }];
    if snapshot.settings.config_directory.is_some() {
        buttons.push(Action {
            id: "settings-keymap-default",
            label: "Use Default",
            run: Run::Change(SettingsChange::ConfigDirectory(None)),
        });
    }
    vec![
        Item::new(
            "settings-keymap-directory",
            "",
            Control::Path {
                text: snapshot.config_dir.clone(),
                unset: false,
                buttons,
            },
        ),
        Item::hint(
            "settings-keymap-directory-hint",
            "The directory holding keymap.conf and hooks.conf. Changing it reloads both.",
        ),
    ]
}

/// One file's diagnostics (`<id>-diagnostics`) and its Reload (`<id>-reload`).
fn diagnostics(id: &'static str, lines: &[String], reload: fn(&mut ngterm_core::Core)) -> Vec<Item> {
    let (list_id, reload_id): (&'static str, &'static str) = match id {
        "settings-keymap" => ("settings-keymap-diagnostics", "settings-keymap-reload"),
        _ => ("settings-hooks-diagnostics", "settings-hooks-reload"),
    };
    vec![
        Item::new(list_id, "", Control::Diagnostics { lines: lines.to_vec() }),
        Item::new(
            reload_id,
            "",
            Control::Button(Action {
                id: reload_id,
                label: "Reload",
                run: Run::Core(reload),
            }),
        ),
    ]
}
