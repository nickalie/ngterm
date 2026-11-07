//! The `keymap.list` and `hooks.list` read-backs (upstream's `ControlKeymap.swift` and `ControlHooks.swift`): what
//! `keymap.conf` resolved plus what the menu carries, and every `hooks.conf` hook with its live state. Fields are in
//! upstream's declaration order and absent when unset, as its encoder leaves them.

use serde::{Deserialize, Deserializer, Serialize};
use serde_with::skip_serializing_none;

/// One rebindable built-in and the chord the keymap resolves it to.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlKeymapAction {
    /// The action's `keymap.conf` name, e.g. `close_session`.
    pub action: String,
    /// The resolved menu chord in kitty syntax, absent for a keyless action.
    #[serde(default)]
    pub chord: Option<String>,
    /// The monitor-bound alternatives in kitty syntax, absent when there are none.
    #[serde(default)]
    pub alternates: Option<Vec<String>>,
    /// `true` when the resolved chord differs from the shipped default; absent otherwise.
    #[serde(default)]
    pub overridden: Option<bool>,
    /// `true` when the `map` line carried `--repeat` and kept a monitor-bound sequence; absent otherwise.
    #[serde(default)]
    pub repeats: Option<bool>,
}

/// One `command` line from `keymap.conf`.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlKeymapCommand {
    pub name: String,
    /// The raw kitty-syntax shortcut, absent for a palette-only command.
    #[serde(default)]
    pub shortcut: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub error_hud: bool,
    /// A HUD position's canonical spelling (`center` unless the line said otherwise).
    #[serde(
        default = "default_error_position",
        deserialize_with = "null_as_default_error_position"
    )]
    pub error_position: String,
    /// `left` or `right`, absent unless the line named one.
    #[serde(default)]
    pub error_pane: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub repeats: bool,
}

fn default_error_position() -> String {
    "center".to_owned()
}

/// Reads an explicit `null` as the type's default, as an omitted field reads (upstream's
/// `decodeIfPresent(...) ?? default`).
fn null_as_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(deserializer: D) -> Result<T, D::Error> {
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}

/// [`null_as_default`] for `errorPosition`, whose default is `center`.
fn null_as_default_error_position<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::deserialize(deserializer)?.unwrap_or_else(default_error_position))
}

/// A `keymap.conf` or `hooks.conf` parse problem. `line` is 1-based; `0` belongs to no single line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlKeymapDiagnostic {
    pub line: i64,
    pub message: String,
}

/// A live menu item carrying an accelerator.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlKeymapMenuItem {
    /// The owning top-level menu's title, e.g. `File`.
    pub menu: String,
    pub title: String,
    /// The chord in kitty syntax, comparable with an action's `chord`.
    pub chord: String,
    /// What the item invokes: the action name of one of ngterm's own items.
    #[serde(default)]
    pub selector: Option<String>,
    /// `false` while the item is disabled (its chord inert); absent when enabled.
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// The `keymap.list` payload.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlKeymap {
    /// The `keymap.conf` that produced this.
    pub path: String,
    pub actions: Vec<ControlKeymapAction>,
    pub commands: Vec<ControlKeymapCommand>,
    pub diagnostics: Vec<ControlKeymapDiagnostic>,
    /// The live menu accelerators; absent when the host could not read a menu.
    #[serde(default)]
    pub menu: Option<Vec<ControlKeymapMenuItem>>,
}

/// The `hooks.list` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlHooks {
    pub path: String,
    pub diagnostics: Vec<ControlKeymapDiagnostic>,
    pub hooks: Vec<ControlHookEntry>,
}

/// One hook's definition and live state. `runningPid`/`elapsedSeconds` are absent while idle; `lastFailure` until a
/// run fails and again after the next clean one; `retired` is `true` for a hook removed from the file whose child
/// still runs.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlHookEntry {
    pub kind: String,
    pub command: String,
    pub line: i64,
    #[serde(default)]
    pub running_pid: Option<i32>,
    #[serde(default)]
    pub elapsed_seconds: Option<f64>,
    pub pending: i64,
    pub dropped: u64,
    #[serde(default)]
    pub last_failure: Option<String>,
    #[serde(default)]
    pub retired: Option<bool>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_minimal_command_decodes_with_the_default_hud_settings() {
        let command: ControlKeymapCommand = serde_json::from_str(r#"{"name":"Palette Only"}"#).unwrap();
        assert_eq!(
            command,
            ControlKeymapCommand {
                name: "Palette Only".to_owned(),
                shortcut: None,
                error_hud: false,
                error_position: "center".to_owned(),
                error_pane: None,
                repeats: false,
            }
        );
        assert_eq!(
            serde_json::to_string(&command).unwrap(),
            r#"{"name":"Palette Only","errorHud":false,"errorPosition":"center","repeats":false}"#
        );
    }

    #[test]
    fn explicit_command_settings_survive_the_wire() {
        let wire = json!({
            "name": "Grow", "shortcut": "ctrl+a>x", "errorHud": true,
            "errorPosition": "bottom-right", "errorPane": "right", "repeats": true
        });
        let command: ControlKeymapCommand = serde_json::from_value(wire).unwrap();
        assert_eq!(command.name, "Grow");
        assert_eq!(command.shortcut.as_deref(), Some("ctrl+a>x"));
        assert!(command.error_hud);
        assert_eq!(command.error_position, "bottom-right");
        assert_eq!(command.error_pane.as_deref(), Some("right"));
        assert!(command.repeats);
        assert_eq!(
            serde_json::to_string(&command).unwrap(),
            r#"{"name":"Grow","shortcut":"ctrl+a>x","errorHud":true,"errorPosition":"bottom-right","errorPane":"right","repeats":true}"#
        );
    }

    /// Upstream's `init(from:)` reads each HUD setting with `decodeIfPresent(...) ?? default`, so an explicit `null`
    /// decodes as the default, as an omitted field does.
    #[test]
    fn a_null_hud_setting_decodes_as_its_default() {
        let command: ControlKeymapCommand = serde_json::from_str(
            r#"{"name":"plain","shortcut":null,"errorHud":null,"errorPosition":null,"errorPane":null,"repeats":null}"#,
        )
        .unwrap();
        assert_eq!(command, serde_json::from_str(r#"{"name":"plain"}"#).unwrap());
        assert_eq!(command.error_position, "center");
        // Only null is lenient: a mistyped setting still fails, as upstream's typed decode does.
        for wrong in [
            r#"{"name":"plain","errorHud":"yes"}"#,
            r#"{"name":"plain","errorPosition":1}"#,
        ] {
            assert!(serde_json::from_str::<ControlKeymapCommand>(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn absent_fields_are_omitted_and_order_is_upstreams() {
        let action = ControlKeymapAction {
            action: "new_session".to_owned(),
            chord: Some("ctrl+shift+f12".to_owned()),
            alternates: Some(vec!["ctrl+shift+f10".to_owned()]),
            overridden: Some(true),
            repeats: None,
        };
        assert_eq!(
            serde_json::to_string(&action).unwrap(),
            r#"{"action":"new_session","chord":"ctrl+shift+f12","alternates":["ctrl+shift+f10"],"overridden":true}"#
        );
        let command = ControlKeymapCommand {
            name: "plain".to_owned(),
            shortcut: None,
            error_hud: false,
            error_position: "center".to_owned(),
            error_pane: None,
            repeats: false,
        };
        assert_eq!(
            serde_json::to_value(&command).unwrap(),
            json!({"name": "plain", "errorHud": false, "errorPosition": "center", "repeats": false})
        );
    }

    #[test]
    fn a_hook_row_reads_back_its_live_fields() {
        let row = ControlHookEntry {
            kind: "session.created".to_owned(),
            command: "true".to_owned(),
            line: 2,
            running_pid: Some(42),
            elapsed_seconds: Some(0.5),
            pending: 1,
            dropped: 0,
            last_failure: None,
            retired: Some(true),
        };
        assert_eq!(
            serde_json::to_string(&row).unwrap(),
            r#"{"kind":"session.created","command":"true","line":2,"runningPid":42,"elapsedSeconds":0.5,"pending":1,"dropped":0,"retired":true}"#
        );
    }
}
