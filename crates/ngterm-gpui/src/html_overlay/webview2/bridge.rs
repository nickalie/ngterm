//! ngterm's scripts in a `WebView2` page and what they post back. `WebView2` has no script worlds and no message
//! handler with a reply (`WebKit`'s `messageHandlers`), so ngterm's scripts run in the page's own world from one
//! closure that holds what the page must not reach:
//!
//! - a nonce, made per page, that every message ngterm's script posts carries, built by string concatenation so no
//!   page code (a `toJSON` on `Object.prototype`, a replaced `JSON.stringify`) ever sees it, and the web view's own
//!   `postMessage` and `addEventListener` taken before the page's scripts run. A message without the nonce, from the
//!   page itself or from a frame, is dropped (upstream refuses frames' requests; a frame never runs this script).
//! - the bridge's handler (upstream's `messageHandlers.ngterm`), whose `postMessage` returns a promise the host's
//!   reply settles: the reply comes back as a web message (`PostWebMessageAsJson`), which only the browser can raise
//!   as a trusted event. Core's adapter and `--js` relay run over it unchanged but for the name they reach it by.
//! - the user's input, real presses and keys only (`isTrusted`, which page code cannot fake), reported for
//!   auto-follow and to lift a declined hand-off's silence (upstream's `onUserInput`).
//!
//! With the page's JavaScript off (`IsScriptEnabled`), `WebView2` still runs these scripts and nothing of the page's.
//! The guard script, in every frame, keeps a page from what the WPE host refuses natively: file choosers and the File
//! System Access pickers, and files pasted onto a page (dropped ones are refused natively, `native.rs`).

use ngterm_core::html::bridge::{ADAPTER_SCRIPT, HELPER_SCRIPT, RELAY_SCRIPT};
use serde_json::Value;

/// How core's scripts reach the bridge's handler on `WebKit`; here the closure's own name for it.
const WEBKIT_HANDLER: &str = "window.webkit.messageHandlers.ngterm";
const HANDLER: &str = "ngtermHandler";

/// The script that reports the user's input and, on a themed page, carries the bridge (module docs), for the top
/// frame only; `nonce` is the page's.
#[must_use]
pub fn page_script(nonce: &str, themed: bool, javascript: bool) -> String {
    let mut script = format!(
        r#"(() => {{
  if (window.top !== window || !window.chrome || !window.chrome.webview) return;
  const webview = window.chrome.webview;
  const post = webview.postMessage.bind(webview);
  const listen = webview.addEventListener.bind(webview);
  const stringify = JSON.stringify;
  const nonce = '{nonce}';
  const send = (kind, rest) => post('{{"n":"' + nonce + '","k":"' + kind + '"' + rest + '}}');
  const report = (click) => (event) => {{ if (event.isTrusted) send('input', ',"click":' + click); }};
  window.addEventListener('pointerdown', report(true), true);
  window.addEventListener('keydown', report(false), true);
"#
    );
    if themed {
        script.push_str(&format!(
            r#"  const waiting = new Map();
  let next = 0;
  listen('message', (event) => {{
    const data = event.data;
    if (!event.isTrusted || !data || data.ngterm !== 'reply' || !waiting.has(data.id)) return;
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
        ));
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

/// A `--js` file page's `ngterm.request` (core's helper), in the top frame.
#[must_use]
pub fn helper_script() -> String {
    format!("if (window.top === window) {HELPER_SCRIPT}")
}

/// The guard every frame of every page runs (module docs).
pub const GUARD_SCRIPT: &str = r"(() => {
  const isFile = (target) => target instanceof HTMLInputElement && target.type === 'file';
  window.addEventListener('click', (event) => {
    const path = event.composedPath ? event.composedPath() : [event.target];
    if (path.some(isFile)) event.preventDefault();
  }, true);
  const showPicker = HTMLInputElement.prototype.showPicker;
  if (showPicker) {
    HTMLInputElement.prototype.showPicker = function () {
      if (isFile(this)) throw new DOMException('file choosers are not available', 'NotAllowedError');
      return showPicker.call(this);
    };
  }
  for (const name of ['showOpenFilePicker', 'showSaveFilePicker', 'showDirectoryPicker']) {
    try { delete window[name]; } catch (error) {}
    try { Object.defineProperty(window, name, {value: undefined, configurable: false}); } catch (error) {}
  }
  window.addEventListener('paste', (event) => {
    const data = event.clipboardData;
    if (data && data.files && data.files.length) {
      event.preventDefault();
      event.stopImmediatePropagation();
    }
  }, true);
})();";

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
    if value.get("n")?.as_str()? != nonce {
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

/// The web message that settles bridge request `id` with `answer`: its result, or its rejection's text.
#[must_use]
pub fn reply(id: u64, answer: Result<Value, String>) -> String {
    let message = match answer {
        Ok(result) => serde_json::json!({"ngterm": "reply", "id": id, "result": result}),
        Err(error) => serde_json::json!({"ngterm": "reply", "id": id, "error": error}),
    };
    message.to_string()
}

/// A fresh nonce for a page: 128 random bits, as hex.
#[must_use]
pub fn nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_counts_only_with_the_page_nonce() {
        let nonce = nonce();
        let bridge =
            format!(r#"{{"n":"{nonce}","k":"bridge","id":3,"body":{{"cmd":"notify","args":{{"text":"hi"}}}}}}"#);
        let Some(Message::Bridge { id, body }) = parse(&bridge, &nonce) else {
            panic!("a bridge request");
        };
        assert_eq!(id, 3);
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap(),
            serde_json::json!({"cmd": "notify", "args": {"text": "hi"}})
        );
        let input = format!(r#"{{"n":"{nonce}","k":"input","click":true}}"#);
        assert_eq!(parse(&input, &nonce), Some(Message::Input { click: true }));
        assert_eq!(parse(&input, "other"), None, "another page's or a forged nonce");
        assert_eq!(parse(r#"{"k":"input","click":true}"#, &nonce), None, "no nonce");
        assert_eq!(parse("not json", &nonce), None);
        assert_eq!(parse(&format!(r#"{{"n":"{nonce}","k":"other"}}"#), &nonce), None);
    }

    #[test]
    fn a_reply_settles_its_request() {
        let ok: Value = serde_json::from_str(&reply(7, Ok(serde_json::json!({"text": "done"})))).unwrap();
        assert_eq!(
            ok,
            serde_json::json!({"ngterm": "reply", "id": 7, "result": {"text": "done"}})
        );
        let refused: Value = serde_json::from_str(&reply(8, Err("page closed".to_owned()))).unwrap();
        assert_eq!(
            refused,
            serde_json::json!({"ngterm": "reply", "id": 8, "error": "page closed"})
        );
    }

    #[test]
    fn the_page_script_holds_the_bridge_only_on_a_themed_page() {
        let nonce = nonce();
        let url_page = page_script(&nonce, false, true);
        assert!(url_page.contains(&nonce) && url_page.contains("'input'"));
        assert!(!url_page.contains(HANDLER), "a URL page gets no bridge");
        let file_page = page_script(&nonce, true, false);
        assert!(file_page.contains("data-ngterm") && file_page.contains(HANDLER));
        assert!(
            !file_page.contains("window.webkit"),
            "core's scripts reach the closure's handler"
        );
        assert!(!file_page.contains("'request'"), "no relay without --js");
        assert!(
            page_script(&nonce, true, true).contains("data.ngterm !== 'request'"),
            "the --js relay"
        );
        assert!(helper_script().starts_with("if (window.top === window)"));
    }
}
