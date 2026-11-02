//! Exercise the Windows transport's host-neutral bridge protocol on Linux as well.
#[cfg(target_os = "linux")]
#[path = "../src/html_overlay/webview2/bridge.rs"]
#[expect(dead_code, reason = "the native guard constant is installed by the platform host")]
mod bridge;
