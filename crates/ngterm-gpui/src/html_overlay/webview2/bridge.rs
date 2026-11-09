//! ngterm's ordinary control scripts in a `WebView2` page and what they post back. Its document-created-script API
//! has no named worlds or reply-capable message handlers (`WebKit`'s `messageHandlers`), so the bridge runs in the page's own world from one
//! closure that holds what the page must not reach:
//!
//! - separate random credentials for posts and replies. Page-world replies may be observable by page code on
//!   `WebView2`, so a reply never discloses the credential used for input reports. This is defence in depth in the
//!   page's JavaScript realm, not an isolated-world boundary. Native navigation and grants enforce access policy.
//! - on macOS, requests use `WebKit`'s reply-capable main-frame message handler; replies settle its native promise
//!   directly instead of passing through page-visible window events or wry's script queue.
//! - the bridge's handler (upstream's `messageHandlers.ngterm`), whose `postMessage` returns a promise the host's
//!   reply settles: the reply comes back as a web message (`PostWebMessageAsJson`) carrying a separate reply credential, so a reply
//!   the page forged (a `message` event it dispatched itself, which `WebView2`'s own does not tell apart by
//!   `isTrusted`) settles nothing. Core's adapter and `--js` relay run over it unchanged but for the name they reach it
//!   by.
//! - the user's input, real presses and keys only (`isTrusted`, which page code cannot fake), reported for
//!   auto-follow and to lift a declined hand-off's silence (upstream's `onUserInput`). On Windows a press also says
//!   which key it was and when, as no key event reports a typed character (`keys`): it ends a half-typed leader.
//! - on Windows, the keys of a live leader or repeat window (`keys`): the host tells the script which keys would
//!   advance or complete the sequence ([`live_keys`]) and until when, the capture listener stops those and only those
//!   and reports them; their autorepeats and releases stay consumed after the sequence ends. With no sequence live it
//!   stops no fresh press. The script's word can lag the host's: a stopped key
//!   the keymap no longer takes goes back into the page ([`redelivery`]) so it is never lost. That script runs in the
//!   top frame alone, so a tail typed inside an iframe goes on reaching the frame.
//!
//! With the page's JavaScript off (`IsScriptEnabled`), `WebView2` still runs these scripts and nothing of the page's.
//! The guard script ([`GUARD_SCRIPT`]), in every frame, keeps a page from what the WPE host refuses natively: file
//! choosers and the File System Access pickers, and files pasted onto a page (dropped ones are refused natively,
//! `native.rs`). `WebView2` offers no native refusal of a file chooser: no event before one opens (webview2-com
//! 0.39.1's API has `SaveAsUIShowing`, the page's own Save As, and `PermissionRequested`'s `FILE_READ_WRITE`, which
//! comes after a File System Access picker, not before), and no setting that turns them off. The guard runs in the
//! page world to replace the APIs authored code calls; an isolated context would see no more of a page's events.
//! Markdown and text pages alone use the separate CDP named-world renderer (`native::add_isolated_document_script`),
//! with no bridge.
//! The guard is defence in depth in the page's own world, not a boundary: it takes what it calls before any page
//! script runs, but a page can still open a chooser through a realm it did not run in (a same-origin frame
//! `WebView2` injects no script into, such as a fresh `about:blank` one, whose own `HTMLElement.prototype.click`
//! clicks a detached file input), and a user's click on a file input inside a closed shadow root escapes it when the
//! root was declared in markup (`<template shadowrootmode="closed">`), or when a page's listener stops the click
//! before it reaches the root. The user still has to pick a file in the dialog.

use std::fmt::Write as _;

use ngterm_core::html::bridge::{ADAPTER_SCRIPT, HELPER_SCRIPT, RELAY_SCRIPT};
use serde_json::Value;

/// How core's scripts reach the bridge's handler on `WebKit`; here the closure's own name for it.
const WEBKIT_HANDLER: &str = "window.webkit.messageHandlers.ngterm";
const HANDLER: &str = "ngtermHandler";

/// The script that reports the user's input and, on a themed page, carries the bridge (module docs), for the top
/// frame only; `nonce` holds the page's independent post and reply credentials.
#[must_use]
pub fn page_script(nonce: &str, themed: bool, javascript: bool) -> String {
    let (request_nonce, response_nonce) = credentials(nonce);
    let mut script = format!(
        r#"(() => {{
  {transport}
  const stringify = JSON.stringify;
  {typed}
  const nonce = '{request_nonce}';
  const responseNonce = '{response_nonce}';
  const send = (kind, rest) => post('{{"n":"' + nonce + '","k":"' + kind + '"' + rest + '}}', kind);
  const report = (click) => (event) => {{ if (event.isTrusted) send('input', ',"click":' + click + (click ? '' : typed(event))); }};
  {keydown}
  window.addEventListener('pointerdown', report(true), true);
  window.addEventListener('keydown', keydown, true);
"#,
        transport = transport(),
        typed = typed(),
        keydown = keydown(),
    );
    if themed {
        let _ = write!(
            script,
            r#"  const waiting = new Map();
  let next = 0;
  listen('message', (event) => {{
    const data = event.data;
    if (!data || data.ngterm !== 'reply' || data.n !== responseNonce || !waiting.has(data.id)) return;
    const {{resolve, reject}} = waiting.get(data.id);
    waiting.delete(data.id);
    if (typeof data.error === 'string') reject(new Error(data.error)); else resolve(data.result);
  }});
  const {HANDLER} = Object.freeze({{postMessage: (body) => new Promise((resolve, reject) => {{
    let text;
    try {{ text = stringify(body); }} catch (error) {{ reject(error); return; }}
    if (typeof text !== 'string') {{ reject(new Error('invalid request')); return; }}
    const id = ++next;
    waiting.set(id, {{resolve, reject}});
    send('bridge', ',"id":' + id + ',"body":' + text);
  }})}});
"#
        );
        if javascript {
            script.push_str(&RELAY_SCRIPT.replace(WEBKIT_HANDLER, HANDLER));
            script.push('\n');
        }
        script.push_str(&ADAPTER_SCRIPT.replace(WEBKIT_HANDLER, HANDLER));
        script.push('\n');
    }
    script.push_str("})();");
    script
}

/// Host-specific transports are selected explicitly, so edits to the Windows setup cannot disable macOS.
fn transport() -> &'static str {
    #[cfg(not(target_os = "macos"))]
    {
        r"if (window.top !== window || !window.chrome || !window.chrome.webview) return;
  const webview = window.chrome.webview;
  const nativePost = webview.postMessage.bind(webview);
  const post = (body, kind) => nativePost(body);
  const listen = webview.addEventListener.bind(webview);"
    }
    #[cfg(target_os = "macos")]
    {
        r"if (window.top !== window || !window.ipc || !window.webkit.messageHandlers.ngtermBridge) return;
  const inputPost = window.ipc.postMessage.bind(window.ipc);
  const handler = window.webkit.messageHandlers.ngtermBridge;
  const bridgePost = handler.postMessage.bind(handler);
  const then = Function.call.bind(Promise.prototype.then);
  const parse = JSON.parse;
  let receive = () => {};
  const listen = (kind, callback) => { receive = callback; };
  const post = (body, kind) => {
    if (kind !== 'bridge') { inputPost(body); return; }
    const id = parse(body).id;
    then(bridgePost(body), data => receive({data}), error => receive({data: {
      ngterm: 'reply', n: responseNonce, id, error: String(error)
    }}));
  };"
    }
}

/// What a keydown's report adds on Windows: the key and when it went down, in the system clock's milliseconds since the
/// epoch, for a fresh press with no Control or Alt held. `AcceleratorKeyPressed` reports the keys that have those
/// (`keys`), and upstream's monitor leaves an autorepeat alone. macOS's monitor hears every press itself.
fn typed() -> &'static str {
    #[cfg(not(target_os = "macos"))]
    {
        r#"const {floor} = Math;
  const {now} = Date;
  const sinceOrigin = performance.now.bind(performance);
  const stampOf = Function.call.bind(Object.getOwnPropertyDescriptor(Event.prototype, 'timeStamp').get);
  const pressedAt = (event) => floor(now() - (sinceOrigin() - stampOf(event)));
  const typed = (event) => event.repeat || event.ctrlKey || event.altKey ? '' :
    ',"key":' + stringify(event.key || '') + ',"at":' + pressedAt(event);"#
    }
    #[cfg(target_os = "macos")]
    {
        "const typed = () => '';"
    }
}

/// The keydown listener. On Windows it stops the keys a live sequence takes ([`live_keys`]), and a stopped key's
/// autorepeats until its release, and reports them instead of the page getting them; everything else is the page's and
/// is reported as `input`. A key that an input method or a dead key is composing stays the page's too. What it reads of
/// the event it took before the page's scripts ran, as the guard does: an event's `isTrusted` cannot be faked, but its
/// other fields can be replaced on the prototype. macOS's monitor takes these keys itself.
fn keydown() -> &'static str {
    #[cfg(not(target_os = "macos"))]
    {
        r#"const keyField = (name) => Function.call.bind(Object.getOwnPropertyDescriptor(KeyboardEvent.prototype, name).get);
  const [virtualKey, repeated, keyName, codeName, shifted, ctrl, alt, meta, composing] =
    ['keyCode', 'repeat', 'key', 'code', 'shiftKey', 'ctrlKey', 'altKey', 'metaKey', 'isComposing'].map(keyField);
  const prevent = Function.call.bind(Event.prototype.preventDefault);
  const halt = Function.call.bind(Event.prototype.stopImmediatePropagation);
  const stopped = new Set();
  let live = null;
  let until = 0;
  listen('message', (event) => {
    const data = event.data;
    if (!data || data.ngterm !== 'keys' || data.n !== responseNonce) return;
    live = data.keys.length ? new Set(data.keys) : null;
    until = sinceOrigin() + data.ms;
  });
  const stops = (event) => {
    const key = virtualKey(event);
    if (repeated(event)) return stopped.has(key);
    stopped.delete(key);
    if (!live || ctrl(event) || alt(event) || meta(event) || composing(event) || keyName(event) === 'Dead') return false;
    if (sinceOrigin() >= until) { live = null; return false; }
    return live.has(key | (shifted(event) ? 0x100 : 0));
  };
  const reportInput = report(false);
  const keydown = (event) => {
    if (!event.isTrusted || !stops(event)) { reportInput(event); return; }
    prevent(event);
    halt(event);
    stopped.add(virtualKey(event));
    send('key', ',"vk":' + virtualKey(event) + ',"shift":' + shifted(event) + ',"repeat":' + repeated(event) +
      ',"key":' + stringify(keyName(event)) + ',"code":' + stringify(codeName(event)) + ',"at":' + pressedAt(event));
  };
  window.addEventListener('keyup', (event) => {
    if (event.isTrusted && stopped.delete(virtualKey(event))) { prevent(event); halt(event); }
  }, true);"#
    }
    #[cfg(target_os = "macos")]
    {
        "const keydown = report(false);"
    }
}

fn credentials(nonce: &str) -> (&str, &str) {
    nonce.split_once(':').unwrap_or((nonce, nonce))
}

/// A `--js` file page's `ngterm.request` (core's helper), in the top frame.
#[must_use]
pub fn helper_script() -> String {
    format!("if (window.top === window) {HELPER_SCRIPT}")
}

/// The guard every frame of every page runs (module docs). It takes every function it calls before the page's
/// scripts run, each bound to `call` so that no later property read reaches the page's replacements: a page replacing
/// `Event.prototype.preventDefault`, `composedPath`, `Array.prototype.some`, an input's `type` or
/// `HTMLInputElement[Symbol.hasInstance]` afterwards changes nothing. An input is a file input by its own `type`
/// getter, whose brand check refuses anything but an input element. A detached input's click never reaches the
/// window's listener, so `click()` and `dispatchEvent` of a `click` (Chromium runs a file input's activation for a
/// page's synthetic click, and for no other untrusted event) on a file input are refused, and so is its
/// `showPicker()`; a click inside a closed shadow root is hidden from the window's `composedPath`, so a root made by
/// `attachShadow` gets the window's listener too. The page only ever sees the guarded methods, and replacing them gives
/// it no way back to the originals in this realm. Its limits: the module docs.
pub const GUARD_SCRIPT: &str = include_str!("guard.js");

/// What ngterm's script in a page posted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// The user pressed (`click`) or typed in the page; `typed_at` is when, for a key that can end a half-typed leader
    /// (`typed`), in milliseconds since the epoch.
    Input { click: bool, typed_at: Option<u64> },
    /// A bridge request: its reply id and the JSON the page sent.
    Bridge { id: u64, body: String },
    /// A key the script stopped for a live sequence ([`keydown`]): the keymap's now, or given back to the page.
    #[cfg(not(target_os = "macos"))]
    Key(StoppedKey),
}

/// A key the page's script kept from the page, as it reported it.
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoppedKey {
    pub virtual_key: u32,
    pub shift: bool,
    pub repeat: bool,
    /// The event's `key` and `code`, which giving it back to the page needs ([`redelivery`]).
    pub key: String,
    pub code: String,
    /// When it went down, in milliseconds since the epoch.
    pub at: u64,
}

/// What [`live_keys`] adds to a virtual key for a press with Shift held; the script's `0x100`.
#[cfg(not(target_os = "macos"))]
pub const SHIFTED: u32 = 0x100;

/// The message `raw` carries when ngterm's script posted it with `nonce`; `None` for anything else (the page's own
/// messages, a frame's, a malformed one).
#[must_use]
pub fn parse(raw: &str, nonce: &str) -> Option<Message> {
    let value: Value = serde_json::from_str(raw).ok()?;
    if value.get("n")?.as_str()? != credentials(nonce).0 {
        return None;
    }
    match value.get("k")?.as_str()? {
        "input" => Some(Message::Input {
            click: value.get("click")?.as_bool()?,
            typed_at: typed_at(&value),
        }),
        "bridge" => Some(Message::Bridge {
            id: value.get("id")?.as_u64()?,
            body: value.get("body")?.to_string(),
        }),
        #[cfg(not(target_os = "macos"))]
        "key" => Some(Message::Key(stopped_key(&value)?)),
        _ => None,
    }
}

/// The stopped key the report `value` names.
#[cfg(not(target_os = "macos"))]
fn stopped_key(value: &Value) -> Option<StoppedKey> {
    Some(StoppedKey {
        virtual_key: u32::try_from(value.get("vk")?.as_u64()?).ok()?,
        shift: value.get("shift")?.as_bool()?,
        repeat: value.get("repeat")?.as_bool()?,
        key: value.get("key")?.as_str()?.to_owned(),
        code: value.get("code")?.as_str()?.to_owned(),
        at: value.get("at")?.as_u64()?,
    })
}

/// When the report `value` says a key that can end a half-typed leader went down: a bare modifier is the next chord
/// being typed, and a key no name is known for names no chord either.
fn typed_at(value: &Value) -> Option<u64> {
    let key = value.get("key")?.as_str()?;
    let names_no_key = matches!(
        key,
        "" | "Unidentified"
            | "Alt"
            | "AltGraph"
            | "CapsLock"
            | "Control"
            | "Fn"
            | "FnLock"
            | "Hyper"
            | "Meta"
            | "NumLock"
            | "OS"
            | "ScrollLock"
            | "Shift"
            | "Super"
            | "Symbol"
            | "SymbolLock"
    );
    if names_no_key { None } else { value.get("at")?.as_u64() }
}

/// The web message that settles bridge request `id` of the page signing with `nonce` with `answer`: its result, or
/// its rejection's text.
#[must_use]
pub fn reply(id: u64, nonce: &str, answer: Result<Value, String>) -> String {
    let nonce = credentials(nonce).1;
    let message = match answer {
        Ok(result) => serde_json::json!({"ngterm": "reply", "n": nonce, "id": id, "result": result}),
        Err(error) => serde_json::json!({"ngterm": "reply", "n": nonce, "id": id, "error": error}),
    };
    message.to_string()
}

/// The web message that tells the script of the page signing with `nonce` which `keys` a live sequence takes for the
/// next `ms` milliseconds: virtual keys, each with [`SHIFTED`] when it is the press with Shift; none when no sequence is
/// live.
#[cfg(not(target_os = "macos"))]
#[must_use]
pub fn live_keys(nonce: &str, keys: &[u32], ms: u64) -> String {
    serde_json::json!({"ngterm": "keys", "n": credentials(nonce).1, "keys": keys, "ms": ms}).to_string()
}

/// The `Input.dispatchKeyEvent` calls that give `key` back to the page: a trusted press and release, so a text field
/// types it and a page's own listeners see it. A stopped key's real press has already gone, so this one is never an
/// autorepeat.
#[cfg(not(target_os = "macos"))]
#[must_use]
pub fn redelivery(key: &StoppedKey) -> [Value; 2] {
    let text = match key.key.as_str() {
        "Enter" => Some("\r".to_owned()),
        name if name.chars().count() == 1 => Some(name.to_owned()),
        _ => None,
    };
    let event = |kind: &str| {
        serde_json::json!({
            "type": kind,
            "key": key.key,
            "code": key.code,
            "windowsVirtualKeyCode": key.virtual_key,
            "modifiers": if key.shift { 8 } else { 0 },
        })
    };
    let mut press = event(if text.is_some() { "keyDown" } else { "rawKeyDown" });
    if let Some(text) = text {
        press["text"] = Value::String(text);
    }
    [press, event("keyUp")]
}

/// Core's theme script as a `WebView2` document runs it: at document creation the document has no root element yet
/// (`WebKit` runs its start scripts once it has one), so the style goes in once the root is there, before anything
/// the document holds.
#[must_use]
pub fn theme_script(script: &str) -> String {
    format!(
        "(() => {{\n  const apply = () => {{ {script} }};\n  if (document.documentElement) {{ apply(); return; }}\n  \
         new MutationObserver((records, observer) => {{\n    if (!document.documentElement) return;\n    \
         observer.disconnect();\n    apply();\n  }}).observe(document, {{childList: true}});\n}})();"
    )
}

/// Independent random 128-bit credentials for a page's posts and replies.
#[must_use]
pub fn nonce() -> String {
    format!("{}:{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_counts_only_with_the_page_nonce() {
        let credential = nonce();
        let nonce = credentials(&credential).0;
        let bridge =
            format!(r#"{{"n":"{nonce}","k":"bridge","id":3,"body":{{"cmd":"notify","args":{{"text":"hi"}}}}}}"#);
        let Some(Message::Bridge { id, body }) = parse(&bridge, &credential) else {
            panic!("a bridge request");
        };
        assert_eq!(id, 3);
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap(),
            serde_json::json!({"cmd": "notify", "args": {"text": "hi"}})
        );
        let input = format!(r#"{{"n":"{nonce}","k":"input","click":true}}"#);
        assert_eq!(
            parse(&input, &credential),
            Some(Message::Input {
                click: true,
                typed_at: None
            })
        );
        assert_eq!(parse(&input, "other"), None, "another page's or a forged nonce");
        assert_eq!(parse(r#"{"k":"input","click":true}"#, nonce), None, "no nonce");
        assert_eq!(parse("not json", nonce), None);
        assert_eq!(parse(&format!(r#"{{"n":"{nonce}","k":"other"}}"#), nonce), None);
    }

    #[test]
    fn a_keydown_report_says_when_a_key_that_can_end_a_leader_went_down() {
        let credential = nonce();
        let nonce = credentials(&credential).0;
        let typed = |rest: &str| {
            let raw = format!(r#"{{"n":"{nonce}","k":"input","click":false{rest}}}"#);
            let Some(Message::Input { click: false, typed_at }) = parse(&raw, &credential) else {
                panic!("an input report: {rest}");
            };
            typed_at
        };
        for key in ["g", "G", " ", "ю", "Enter", "ArrowLeft", "Dead", "Process"] {
            assert_eq!(typed(&format!(r#","key":"{key}","at":7"#)), Some(7), "{key}");
        }
        for key in [
            "",
            "Unidentified",
            "Alt",
            "AltGraph",
            "CapsLock",
            "Control",
            "Fn",
            "FnLock",
            "Hyper",
            "Meta",
            "NumLock",
            "OS",
            "ScrollLock",
            "Shift",
            "Super",
            "Symbol",
            "SymbolLock",
        ] {
            assert_eq!(typed(&format!(r#","key":"{key}","at":7"#)), None, "{key}");
        }
        assert_eq!(typed(""), None, "a chord's report names no key");
        assert_eq!(typed(r#","key":"g""#), None, "no time");
        assert_eq!(typed(r#","key":"g","at":"soon""#), None);
        assert_eq!(typed(r#","key":7,"at":7"#), None);
    }

    /// The script a page runs on Windows reports the key of a fresh press without Control or Alt, as JSON the host
    /// parses whatever the key is called; macOS's monitor hears every press itself, so its reports stay as they were.
    #[test]
    fn only_windows_pages_say_which_key_they_typed() {
        let script = page_script(&nonce(), false, false);
        let says = script.contains(r#","key":' + stringify(event.key"#);
        assert_eq!(says, !cfg!(target_os = "macos"));
        assert_eq!(
            script.contains("event.repeat || event.ctrlKey || event.altKey"),
            !cfg!(target_os = "macos")
        );
        assert_eq!(script.contains("const typed = () => '';"), cfg!(target_os = "macos"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_stopped_keys_report_names_the_key_and_how_it_was_pressed() {
        let credential = nonce();
        let nonce = credentials(&credential).0;
        let report = format!(
            r#"{{"n":"{nonce}","k":"key","vk":71,"shift":true,"repeat":false,"key":"G","code":"KeyG","at":9}}"#
        );
        assert_eq!(
            parse(&report, &credential),
            Some(Message::Key(StoppedKey {
                virtual_key: 71,
                shift: true,
                repeat: false,
                key: "G".to_owned(),
                code: "KeyG".to_owned(),
                at: 9
            }))
        );
        let forged = format!(r#"{{"n":"{}","k":"key","vk":71}}"#, credentials(&credential).1);
        assert_eq!(
            parse(&forged, &credential),
            None,
            "the reply credential authorizes nothing"
        );
        assert_eq!(
            parse(&format!(r#"{{"n":"{nonce}","k":"key","vk":71}}"#), &credential),
            None
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_host_tells_the_script_the_live_keys_with_the_reply_credential() {
        let credential = nonce();
        let told: Value = serde_json::from_str(&live_keys(&credential, &[0x47, 0x47 | SHIFTED], 1500)).unwrap();
        assert_eq!(
            told,
            serde_json::json!({"ngterm": "keys", "n": credentials(&credential).1, "keys": [71, 327], "ms": 1500})
        );
        let idle: Value = serde_json::from_str(&live_keys(&credential, &[], 0)).unwrap();
        assert_eq!(idle["keys"], serde_json::json!([]));
    }

    /// The script is what stops a key, in a real page's capture phase: the part a unit test can see is what it reads.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_script_stops_live_keys_and_reports_them_as_trusted_only() {
        let script = page_script(&nonce(), false, false);
        assert!(script.contains("data.ngterm !== 'keys' || data.n !== responseNonce"));
        assert!(script.contains("if (!event.isTrusted || !stops(event))"));
        assert!(script.contains("window.addEventListener('keydown', keydown, true)"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_macos_page_stops_no_key() {
        let script = page_script(&nonce(), false, false);
        assert!(script.contains("const keydown = report(false);"));
        assert!(!script.contains("preventDefault") && !script.contains("stopImmediatePropagation"));
        let credential = nonce();
        let nonce = credentials(&credential).0;
        let report = format!(
            r#"{{"n":"{nonce}","k":"key","vk":71,"shift":false,"repeat":false,"key":"g","code":"KeyG","at":9}}"#
        );
        assert_eq!(parse(&report, &credential), None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_stopped_key_goes_back_to_the_page_as_a_trusted_press_and_release() {
        let stopped = |key: &str, code: &str, virtual_key: u32, shift: bool| StoppedKey {
            virtual_key,
            shift,
            repeat: true,
            key: key.to_owned(),
            code: code.to_owned(),
            at: 0,
        };
        let [press, release] = redelivery(&stopped("G", "KeyG", 71, true));
        assert_eq!(
            press,
            serde_json::json!({
                "type": "keyDown", "key": "G", "code": "KeyG", "windowsVirtualKeyCode": 71, "modifiers": 8, "text": "G"
            })
        );
        assert_eq!(
            release,
            serde_json::json!({
                "type": "keyUp", "key": "G", "code": "KeyG", "windowsVirtualKeyCode": 71, "modifiers": 8
            })
        );
        let [enter, _] = redelivery(&stopped("Enter", "Enter", 13, false));
        assert_eq!(enter["text"], "\r", "Enter types a line break");
        let [backspace, _] = redelivery(&stopped("Backspace", "Backspace", 8, false));
        assert_eq!(backspace["type"], "rawKeyDown", "Backspace types nothing");
        assert!(backspace.get("text").is_none());
    }

    #[test]
    fn a_reply_does_not_authorize_forged_input() {
        let nonce = nonce();
        let reply: Value = serde_json::from_str(&reply(1, &nonce, Ok(Value::Null))).unwrap();
        let forged = serde_json::json!({"n": reply["n"], "k": "input", "click": true});
        assert!(parse(&forged.to_string(), &nonce).is_none());
        assert_ne!(credentials(&nonce).0, credentials(&nonce).1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_uses_a_native_reply_transport() {
        let script = page_script(&nonce(), true, true);
        assert!(script.contains("window.ipc") && script.contains("ngtermBridge"));
        assert!(!script.contains("window.chrome") && !script.contains("window.dispatchEvent"));
    }

    #[test]
    fn a_reply_settles_its_request() {
        let ok: Value = serde_json::from_str(&reply(7, "n1", Ok(serde_json::json!({"text": "done"})))).unwrap();
        assert_eq!(
            ok,
            serde_json::json!({"ngterm": "reply", "n": "n1", "id": 7, "result": {"text": "done"}})
        );
        let refused: Value = serde_json::from_str(&reply(8, "n1", Err("page closed".to_owned()))).unwrap();
        assert_eq!(
            refused,
            serde_json::json!({"ngterm": "reply", "n": "n1", "id": 8, "error": "page closed"})
        );
    }

    #[test]
    fn the_theme_waits_for_the_documents_root() {
        let theme = theme_script("document.documentElement.prepend(style);");
        assert!(theme.contains("if (document.documentElement) { apply(); return; }"));
        assert!(theme.contains("MutationObserver") && theme.contains("document.documentElement.prepend(style);"));
    }

    #[test]
    fn the_page_script_holds_the_bridge_only_on_a_themed_page() {
        let credential = nonce();
        let nonce = credentials(&credential).0;
        let url_page = page_script(&credential, false, true);
        assert!(url_page.contains(nonce) && url_page.contains("'input'"));
        assert!(!url_page.contains(HANDLER), "a URL page gets no bridge");
        let file_page = page_script(&credential, true, false);
        assert!(file_page.contains("data-ngterm") && file_page.contains(HANDLER));
        assert!(
            !file_page.contains(&format!("{WEBKIT_HANDLER};")),
            "core's scripts reach the closure's handler"
        );
        assert!(!file_page.contains("'request'"), "no relay without --js");
        assert!(
            page_script(&credential, true, true).contains("data.ngterm !== 'request'"),
            "the --js relay"
        );
        assert!(helper_script().starts_with("if (window.top === window)"));
    }
}

/// The capture listener on a real Chromium document, with trusted CDP presses and the page's own event handlers.
#[cfg(test)]
#[cfg(windows)]
mod browser_tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use raw_window_handle::{HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::HSTRING;
    use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
        WS_EX_TOOLWINDOW, WS_POPUP,
    };
    use wry::{WebView, WebViewBuilder, WebViewExtWindows as _};

    use super::*;

    const NONCE: &str = "post:reply";
    const RELEASE: &str = "html_overlay::webview2::bridge::browser_tests::release_child";
    const STAMP: &str = "html_overlay::webview2::bridge::browser_tests::timestamp_child";

    struct Parent(windows_sys::Win32::Foundation::HWND);

    impl HasWindowHandle for Parent {
        fn window_handle(&self) -> Result<WindowHandle<'_>, raw_window_handle::HandleError> {
            let handle =
                Win32WindowHandle::new(std::num::NonZeroIsize::new(self.0.expose_provenance().cast_signed()).unwrap());
            // SAFETY: this test owns the hidden window for the web view's lifetime.
            Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
        }
    }

    impl Drop for Parent {
        fn drop(&mut self) {
            // SAFETY: all web views were dropped before their test-owned parent and this thread's COM apartment.
            unsafe { DestroyWindow(self.0) };
            // SAFETY: the apartment was initialized on this thread and all of its web views have been dropped.
            unsafe { CoUninitialize() };
        }
    }

    struct Browser {
        view: WebView,
        reports: Rc<RefCell<Vec<String>>>,
        _parent: Parent,
        _directory: tempfile::TempDir,
    }

    impl Browser {
        fn new() -> Self {
            // SAFETY: this isolated child thread owns the apartment until Parent drops.
            assert!(unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED.cast_unsigned()) } >= 0);
            // SAFETY: a system class, no parent, no menu and no data; the hidden window belongs to this test thread.
            let parent = Parent(unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    windows_sys::core::w!("STATIC"),
                    std::ptr::null(),
                    WS_POPUP,
                    0,
                    0,
                    320,
                    200,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            });
            assert!(!parent.0.is_null());
            let directory = tempfile::tempdir().unwrap();
            let reports = Rc::new(RefCell::new(Vec::new()));
            let sent = Rc::clone(&reports);
            let mut context = wry::WebContext::new(Some(directory.path().to_path_buf()));
            let view = WebViewBuilder::new_with_web_context(&mut context)
                .with_initialization_script(page_script(NONCE, false, false))
                .with_ipc_handler(move |request| sent.borrow_mut().push(request.body().clone()))
                .with_html("<!doctype html><input id=field><script>\
                    window.releases = []; addEventListener('keyup', e => releases.push(e.key));\
                    chrome.webview.addEventListener('message', e => { if (e.data.ngterm === 'keys') ipc.postMessage('told'); });\
                    ipc.postMessage('ready');</script>")
                .build_as_child(&parent).unwrap();
            let browser = Self {
                view,
                reports,
                _parent: parent,
                _directory: directory,
            };
            browser.wait_for("ready");
            browser.cdp(
                "Emulation.setFocusEmulationEnabled",
                &serde_json::json!({"enabled": true}),
            );
            browser.evaluate("field.focus()");
            browser
        }

        fn wait_for(&self, expected: &str) {
            let until = Instant::now() + Duration::from_secs(15);
            while !self.reports.borrow().iter().any(|report| report == expected) {
                assert!(
                    Instant::now() < until,
                    "missing {expected}: {:?}",
                    self.reports.borrow()
                );
                let mut message = MSG::default();
                // SAFETY: only this test thread's messages, read into its own buffer.
                while unsafe { PeekMessageW(&raw mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
                    // SAFETY: the message just read from this thread's queue.
                    unsafe { TranslateMessage(&raw const message) };
                    // SAFETY: dispatching that message on its owning thread.
                    unsafe { DispatchMessageW(&raw const message) };
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn cdp(&self, method: &str, parameters: &Value) {
            let view = self.view.webview();
            let method = HSTRING::from(method);
            let parameters = HSTRING::from(parameters.to_string());
            CallDevToolsProtocolMethodCompletedHandler::wait_for_async_operation(
                Box::new(move |handler| {
                    // SAFETY: the web view is called on its owning apartment thread.
                    unsafe { view.CallDevToolsProtocolMethod(&method, &parameters, &handler) }.map_err(Into::into)
                }),
                Box::new(|result, _| result),
            )
            .unwrap();
        }

        fn evaluate(&self, expression: &str) {
            self.cdp("Runtime.evaluate", &serde_json::json!({"expression": expression}));
        }

        fn tell(&self, keys: &[u32]) {
            self.reports.borrow_mut().clear();
            // SAFETY: a live web view and a valid JSON string, on its apartment thread.
            unsafe {
                self.view
                    .webview()
                    .PostWebMessageAsJson(&HSTRING::from(live_keys(NONCE, keys, 60_000)))
            }
            .unwrap();
            self.wait_for("told");
        }

        fn key(&self, key: &str, code: &str, virtual_key: u32) {
            for event in redelivery(&StoppedKey {
                virtual_key,
                shift: false,
                repeat: false,
                key: key.into(),
                code: code.into(),
                at: 0,
            }) {
                self.cdp("Input.dispatchKeyEvent", &event);
            }
        }
    }

    #[test]
    fn consumed_releases_stay_off_a_real_page() {
        crate::system_window::tests::run_child(RELEASE);
    }

    #[test]
    #[ignore = "run by consumed_releases_stay_off_a_real_page"]
    fn release_child() {
        if !crate::system_window::tests::is_child(RELEASE) {
            return;
        }
        let browser = Browser::new();
        browser.tell(&[0x47]);
        let [press, release] = redelivery(&StoppedKey {
            virtual_key: 0x47,
            shift: false,
            repeat: false,
            key: "g".into(),
            code: "KeyG".into(),
            at: 0,
        });
        browser.cdp("Input.dispatchKeyEvent", &press);
        browser.tell(&[]);
        let mut repeated = press;
        repeated["autoRepeat"] = Value::Bool(true);
        browser.cdp("Input.dispatchKeyEvent", &repeated);
        browser.cdp("Input.dispatchKeyEvent", &release);
        browser.tell(&[0x0D, 0x08]);
        browser.key("Enter", "Enter", 0x0D);
        browser.key("Backspace", "Backspace", 0x08);
        browser.key("x", "KeyX", 0x58);
        browser.evaluate("ipc.postMessage('releases:' + JSON.stringify(releases))");
        browser.evaluate("ipc.postMessage('first-releases-done')");
        browser.wait_for("first-releases-done");
        assert!(
            browser
                .reports
                .borrow()
                .iter()
                .any(|report| report == "releases:[\"x\"]"),
            "a consumed press, its autorepeat and its release stay off the page: {:?}",
            browser.reports.borrow()
        );
        browser.tell(&[]);
        browser.key("g", "KeyG", 0x47);
        browser.evaluate("ipc.postMessage('releases:' + JSON.stringify(releases))");
        browser.wait_for("releases:[\"x\",\"g\"]");
    }

    #[test]
    fn a_page_cannot_replace_a_stopped_keys_timestamp() {
        crate::system_window::tests::run_child(STAMP);
    }

    #[test]
    #[ignore = "run by a_page_cannot_replace_a_stopped_keys_timestamp"]
    fn timestamp_child() {
        if !crate::system_window::tests::is_child(STAMP) {
            return;
        }
        let browser = Browser::new();
        browser.evaluate("Object.defineProperty(Event.prototype, 'timeStamp', {get: () => undefined})");
        browser.tell(&[0x47]);
        browser.key("g", "KeyG", 0x47);
        browser.evaluate("ipc.postMessage('done')");
        browser.wait_for("done");
        let reports = browser.reports.borrow();
        let keys: Vec<_> = reports
            .iter()
            .filter_map(|report| match parse(report, NONCE) {
                Some(Message::Key(key)) => Some(key),
                _ => None,
            })
            .collect();
        assert_eq!(keys.len(), 1, "the stopped key is still a valid report: {reports:?}");
        assert_eq!(keys[0].key, "g");
        assert!(keys[0].at > 0);
    }
}
