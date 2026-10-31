//! The request line: `{cmd, target?, args?}`, one per connection, newline-delimited JSON.

use std::fmt;

use serde::de::value::MapAccessDeserializer;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_with::skip_serializing_none;

use crate::command::Command;

/// Cap on a request line in bytes, newline excluded (upstream's `ControlWire.maxRequestLineBytes`). Over it the
/// server answers `request too large or read failed` and closes the connection; the client checks the same cap
/// before writing.
pub const MAX_REQUEST_LINE_BYTES: usize = 1 << 20;

/// Implements `Serialize` and `Deserialize` for wire records whose derives carry `#[serde(remote = "Self")]`, so they
/// decode from a JSON object only. A derived struct deserializer also takes a positional array
/// (`["session.close", "active", null]`), which upstream's `JSONDecoder` refuses for its keyed `Codable` structs
/// (`ControlRequest`, `ControlArgs` in `agtermCore/Sources/agtermCore/ControlProtocol.swift`); going through
/// `deserialize_map` makes that a decode error. The derived inherent `deserialize` stays lax: decode through the
/// trait (`serde_json::from_str`), never by calling `Request::deserialize` as a path.
macro_rules! object_only_serde {
    ($($ty:ident),+ $(,)?) => {$(
        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                $ty::serialize(self, serializer)
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct ObjectVisitor;

                impl<'de> Visitor<'de> for ObjectVisitor {
                    type Value = $ty;

                    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                        formatter.write_str(concat!("a JSON object (", stringify!($ty), ")"))
                    }

                    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<$ty, A::Error> {
                        $ty::deserialize(MapAccessDeserializer::new(map))
                    }
                }

                deserializer.deserialize_map(ObjectVisitor)
            }
        }
    )+};
}

object_only_serde!(Request, Args, PickItem, AskButton, AskChoice);

/// One control request. A request that is not a JSON object, whose `cmd` is unknown, or whose fields carry a wrong
/// JSON type fails to decode as a whole; unknown keys are ignored.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(remote = "Self")]
pub struct Request {
    pub cmd: Command,
    /// A session or workspace id, `active`, or a unique prefix.
    pub target: Option<String>,
    pub args: Option<Args>,
}

impl Request {
    /// A request with no target and no args.
    #[must_use]
    pub fn new(cmd: Command) -> Self {
        Self {
            cmd,
            target: None,
            args: None,
        }
    }
}

/// The bag of optional command parameters. Each command reads only the fields it needs; absent ones are omitted
/// from the JSON. Key names are the wire's (`paneID`, `workspaceName`, ...).
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(remote = "Self", rename_all = "camelCase")]
pub struct Args {
    pub name: Option<String>,
    pub cwd: Option<String>,
    pub targets: Option<Vec<String>>,
    /// `session.close`: ask for the undo grace and a `result.undoable` answer.
    pub undoable: Option<bool>,
    pub workspace: Option<String>,
    pub workspace_name: Option<String>,
    pub create_workspace: Option<bool>,
    pub collapsed: Option<bool>,
    /// The tint color of `session.tint`/`workspace.tint` mode `set`, and the creation tint of `session.new`,
    /// `session.companion.new` and `workspace.new`.
    pub tint: Option<String>,
    /// The row parts the tint colors (`icon`, `name`).
    pub tint_on: Option<Vec<String>>,
    pub minimized: Option<bool>,
    pub force: Option<bool>,
    pub host: Option<String>,
    pub no_select: Option<bool>,
    pub text: Option<String>,
    pub select: Option<bool>,
    pub mode: Option<String>,
    pub axis: Option<String>,
    pub path: Option<String>,
    pub color: Option<String>,
    pub text_color: Option<String>,
    pub shape: Option<String>,
    pub opacity: Option<f64>,
    pub fit: Option<String>,
    pub position: Option<String>,
    pub repeats: Option<bool>,
    pub pane: Option<String>,
    #[serde(rename = "paneID")]
    pub pane_id: Option<String>,
    pub ratio: Option<f64>,
    pub ratio_delta: Option<f64>,
    pub all: Option<bool>,
    pub lines: Option<i64>,
    pub to: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub run: Option<String>,
    pub kinds: Option<Vec<String>>,
    pub limit: Option<i64>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub command: Option<String>,
    pub wait: Option<bool>,
    pub size_percent: Option<i64>,
    pub full: Option<bool>,
    pub follow: Option<bool>,
    pub message: Option<String>,
    pub detail: Option<String>,
    pub spinner: Option<String>,
    pub hide_after: Option<f64>,
    pub markdown: Option<bool>,
    pub sticky: Option<bool>,
    pub frame: Option<bool>,
    pub items: Option<Vec<PickItem>>,
    pub prompt: Option<String>,
    pub query: Option<String>,
    pub allow_custom: Option<bool>,
    pub selection: Option<String>,
    pub buttons: Option<Vec<AskButton>>,
    pub default_button: Option<String>,
    pub style: Option<String>,
    pub align: Option<String>,
    pub destructive_button: Option<String>,
    /// `ask.open`: the button that resolves `escaped`, as Esc does.
    pub cancel_button: Option<String>,
    /// `ask.open`: `line` or `multiline`, giving the dialog a text field.
    pub input: Option<String>,
    /// `ask.open`: the hint an empty text field shows.
    pub placeholder: Option<String>,
    /// `ask.open`: a radio or checkbox list, in the caller's order.
    pub choices: Option<Vec<AskChoice>>,
    /// `ask.open`: the choice list is checkboxes instead of radio buttons.
    pub multiple: Option<bool>,
    /// `ask.open`: the choice ids selected at open.
    pub checked: Option<Vec<String>>,
    /// `ask.open`: a free-form row after the choice list; the value is its placeholder.
    pub other: Option<String>,
    pub window: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub sidebar_width: Option<f64>,
    pub x: Option<i64>,
    pub y: Option<i64>,
    pub display: Option<i64>,
    pub status: Option<String>,
    pub blink: Option<bool>,
    pub auto_reset: Option<bool>,
    pub sound: Option<String>,
    pub light: Option<String>,
    pub dark: Option<String>,
    pub close: Option<bool>,
    pub font_size: Option<f64>,
    pub auto_size: Option<bool>,
    pub mru: Option<bool>,
    pub html: Option<String>,
    pub current: Option<bool>,
    pub navigation: Option<bool>,
    pub url: Option<String>,
    pub javascript: Option<bool>,
    pub chromeless: Option<bool>,
    pub persistent: Option<bool>,
    pub browse: Option<bool>,
    /// The answer `session.overlay.submit` hands back from a page (empty is a real answer), or the initial text of an
    /// `ask.open` text field.
    pub value: Option<String>,
    pub page: Option<String>,
}

/// One caller-provided choice for `pick.open` (upstream's `ControlPickItem`).
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(remote = "Self")]
pub struct PickItem {
    pub id: String,
    pub label: String,
    pub subtitle: Option<String>,
    /// The row's trailing glyph in `session.status` values; absent or `idle` draws none.
    pub status: Option<String>,
    /// A `#rrggbb` tint for that glyph.
    #[serde(rename = "statusColor")]
    pub status_color: Option<String>,
    /// A `session.status --shape` silhouette for that glyph.
    #[serde(rename = "statusShape")]
    pub status_shape: Option<String>,
}

/// One caller-provided dialog button for `ask.open`.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self")]
pub struct AskButton {
    pub id: String,
    pub label: String,
    /// An optional ASCII letter that activates the button.
    pub hotkey: Option<String>,
}

/// One caller-provided item of an `ask.open` choice list (upstream's `ControlAskChoice`).
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self")]
pub struct AskChoice {
    /// The item's id in `checked` and in the answered `selected` list.
    pub id: String,
    /// The item's one-line text.
    pub label: String,
    /// An optional description drawn dimmed under the label; it may span lines.
    pub subtitle: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::command::{PickCmd, SessionCmd};

    #[test]
    fn bare_request_omits_target_and_args() {
        let json = serde_json::to_value(Request::new(Command::Tree)).unwrap();
        assert_eq!(json, json!({"cmd": "tree"}));
    }

    #[test]
    fn full_request_round_trips() {
        let wire = json!({
            "cmd": "session.new",
            "target": "active",
            "args": {"name": "build", "paneID": "p-1", "noSelect": true, "workspaceName": "w", "ratioDelta": 0.25,
                     "sizePercent": 40, "targets": ["a", "b"]}
        });
        let request: Request = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(request.cmd, Command::Session(SessionCmd::New));
        assert_eq!(request.target.as_deref(), Some("active"));
        let args = request.args.clone().unwrap();
        assert_eq!(args.pane_id.as_deref(), Some("p-1"));
        assert_eq!(args.no_select, Some(true));
        assert_eq!(args.size_percent, Some(40));
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
    }

    #[test]
    fn nested_arg_items_round_trip() {
        let wire = json!({
            "cmd": "pick.open",
            "args": {"items": [{"id": "1", "label": "one"}, {"id": "2", "label": "two", "subtitle": "s"}],
                     "buttons": [{"id": "ok", "label": "OK", "hotkey": "o"}]}
        });
        let request: Request = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(request.cmd, Command::Pick(PickCmd::Open));
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
    }

    /// Upstream's `ControlPickTests.statusRoundTripsAndOmitsWhenNil`.
    #[test]
    fn pick_item_status_round_trips_and_is_omitted_when_none() {
        let item = PickItem {
            id: "s1".to_owned(),
            label: "build".to_owned(),
            status: Some("blocked".to_owned()),
            status_color: Some("#ff8800".to_owned()),
            status_shape: Some("star".to_owned()),
            ..PickItem::default()
        };
        let wire = serde_json::to_value(&item).unwrap();
        assert_eq!(
            wire,
            json!({"id": "s1", "label": "build", "status": "blocked", "statusColor": "#ff8800", "statusShape": "star"})
        );
        let keys: Vec<&str> = wire.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["id", "label", "status", "statusColor", "statusShape"]);
        assert_eq!(serde_json::from_value::<PickItem>(wire).unwrap(), item);
        let plain = PickItem {
            id: "a".to_owned(),
            label: "A".to_owned(),
            ..PickItem::default()
        };
        let text = serde_json::to_string(&plain).unwrap();
        assert!(!text.contains("status"), "{text}");
        assert_eq!(
            serde_json::from_str::<PickItem>(r#"{"id":"a","label":"A"}"#).unwrap(),
            plain
        );
    }

    #[test]
    fn ask_form_args_round_trip_in_upstreams_order() {
        let wire = json!({
            "cmd": "ask.open",
            "args": {"defaultButton": "ok", "destructiveButton": "no", "cancelButton": "cancel", "input": "line",
                     "placeholder": "type", "choices": [{"id": "a", "label": "A", "subtitle": "s\nt"}, {"id": "b", "label": "B"}],
                     "multiple": true, "checked": ["b"], "other": "Something else", "title": "T", "value": "x"}
        });
        let request: Request = serde_json::from_value(wire.clone()).unwrap();
        let args = request.args.clone().unwrap();
        assert_eq!(
            args.choices.as_deref(),
            Some(
                &[
                    AskChoice {
                        id: "a".into(),
                        label: "A".into(),
                        subtitle: Some("s\nt".into())
                    },
                    AskChoice {
                        id: "b".into(),
                        label: "B".into(),
                        subtitle: None
                    },
                ][..]
            )
        );
        assert_eq!(args.cancel_button.as_deref(), Some("cancel"));
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        let Value::Object(map) = serde_json::to_value(args).unwrap() else {
            panic!("args is not an object")
        };
        let keys: Vec<&str> = map.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "title",
                "defaultButton",
                "destructiveButton",
                "cancelButton",
                "input",
                "placeholder",
                "choices",
                "multiple",
                "checked",
                "other",
                "value"
            ]
        );
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"ask.open","args":{"choices":[["a","A",null]]}}"#).is_err());
    }

    #[test]
    fn unknown_arg_keys_are_ignored_and_null_is_absent() {
        let request: Request =
            serde_json::from_str(r#"{"cmd":"version","target":null,"args":{"bogus":1,"name":null}}"#).unwrap();
        assert_eq!(request.target, None);
        assert_eq!(request.args, Some(Args::default()));
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({"cmd": "version", "args": {}})
        );
    }

    #[test]
    fn undecodable_requests_fail_as_a_whole() {
        for line in [
            r#"{"cmd":"nope"}"#,
            r#"{"target":"active"}"#,
            r#"{"cmd":"tree","target":5}"#,
            r#"{"cmd":"session.new","args":{"noSelect":"yes"}}"#,
            r#"{"cmd":"session.text","args":{"lines":"3"}}"#,
            r#"{"cmd":"pick.open","args":{"items":[{"id":"1"}]}}"#,
            r#"["tree"]"#,
        ] {
            assert!(serde_json::from_str::<Request>(line).is_err(), "{line}");
        }
    }

    #[test]
    fn positional_arrays_are_refused_at_every_level() {
        for line in [
            r#"["session.close","active",null]"#,
            r#"["session.close","active",{}]"#,
            r#"{"cmd":"session.new","args":["build"]}"#,
            r#"{"cmd":"pick.open","args":{"items":[["1","one",null]]}}"#,
            r#"{"cmd":"ask.open","args":{"buttons":[["ok","OK",null]]}}"#,
        ] {
            let error = serde_json::from_str::<Request>(line).unwrap_err();
            assert!(error.to_string().contains("expected a JSON object"), "{line}: {error}");
        }
    }

    #[test]
    fn tint_and_undoable_args_round_trip_in_upstreams_order() {
        let wire = json!({
            "cmd": "session.close",
            "target": "a",
            "args": {"targets": ["a", "b"], "undoable": true, "collapsed": true, "tint": "red",
                     "tintOn": ["icon", "name"], "mode": "set"}
        });
        let request: Request = serde_json::from_value(wire.clone()).unwrap();
        let args = request.args.clone().unwrap();
        assert_eq!((args.undoable, args.tint.as_deref()), (Some(true), Some("red")));
        assert_eq!(args.tint_on, Some(vec!["icon".to_owned(), "name".to_owned()]));
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        let Value::Object(map) = serde_json::to_value(args).unwrap() else {
            panic!("args is not an object")
        };
        let keys: Vec<&str> = map.keys().map(String::as_str).collect();
        assert_eq!(keys, ["targets", "undoable", "collapsed", "tint", "tintOn", "mode"]);
        assert_eq!(serde_json::to_string(&Args::default()).unwrap(), "{}");
    }

    #[test]
    fn every_arg_key_uses_the_wire_spelling() {
        let args = Args {
            pane_id: Some(String::new()),
            hide_after: Some(1.5),
            font_size: Some(12.0),
            ..Args::default()
        };
        let Value::Object(map) = serde_json::to_value(args).unwrap() else {
            panic!("args is not an object")
        };
        let keys: Vec<&str> = map.keys().map(String::as_str).collect();
        assert_eq!(keys, ["paneID", "hideAfter", "fontSize"]);
    }
}
