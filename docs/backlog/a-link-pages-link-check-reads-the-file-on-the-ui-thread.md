---
worth: yes
where: crates/ngterm-gpui/src/html_overlay/webview2/page.rs:576
added: 2026-11-22
---
# A link page's link check reads the file on the UI thread on WKWebView and WebView2

On WKWebView and WebView2, a failed read of the page protocol shows as a document. Thus, before a Markdown link page
follows a link, `Page::allows` checks the link through `Page::link_readable`, which calls `policy::link_readable`. That
resolves the path and reads the file up to the Markdown cap to check that it is UTF-8. This occurs in the engine's
synchronous navigation callback on the main thread, for each link click, each Back/Forward step and the first load. On
slow storage, such as a network share, the app stops until the read ends. Upstream's policy does no synchronous
check, and the click path of `links.rs` keeps its file reads on a worker.

A file that changes between this check and the protocol's read also still shows the protocol's error answer as the
page.

Possible fix: cancel the navigation, check the file on a worker, then load the address if the check passes. A
Back/Forward step cannot be replayed after a cancel, so this needs a design for history steps. Another possibility:
make the protocol's failed read go back to the shown document, and remove the synchronous check.

Reported in the revmux review of task #305 (Markdown pages follow local links), round 1.
