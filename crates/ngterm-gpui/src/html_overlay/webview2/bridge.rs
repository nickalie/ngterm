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
//!   auto-follow and to lift a declined hand-off's silence (upstream's `onUserInput`).
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
  const nonce = '{request_nonce}';
  const responseNonce = '{response_nonce}';
  const send = (kind, rest) => post('{{"n":"' + nonce + '","k":"' + kind + '"' + rest + '}}', kind);
  const report = (click) => (event) => {{ if (event.isTrusted) send('input', ',"click":' + click); }};
  window.addEventListener('pointerdown', report(true), true);
  window.addEventListener('keydown', report(false), true);
"#,
        transport = transport(),
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
    /// The user pressed (`click`) or typed in the page.
    Input { click: bool },
    /// A bridge request: its reply id and the JSON the page sent.
    Bridge { id: u64, body: String },
}

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
        }),
        "bridge" => Some(Message::Bridge {
            id: value.get("id")?.as_u64()?,
            body: value.get("body")?.to_string(),
        }),
        _ => None,
    }
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
        assert_eq!(parse(&input, &credential), Some(Message::Input { click: true }));
        assert_eq!(parse(&input, "other"), None, "another page's or a forged nonce");
        assert_eq!(parse(r#"{"k":"input","click":true}"#, nonce), None, "no nonce");
        assert_eq!(parse("not json", nonce), None);
        assert_eq!(parse(&format!(r#"{{"n":"{nonce}","k":"other"}}"#), nonce), None);
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
