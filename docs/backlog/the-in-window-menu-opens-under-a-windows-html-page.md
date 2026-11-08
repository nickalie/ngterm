---
worth: yes
where: crates/ngterm-gpui/src/window.rs:1059
added: 2026-11-08
---
# the in-window menu opens under a Windows HTML page

On Windows a WebView2 page is a native child window, which sits above everything the GPUI window draws. The window
steps its pages aside while a layer it draws covers the content (`html_overlay::set_covered`, computed in
`window.rs` from the alert, the switcher and the dialog/palette/popover layers), but the title bar's app menu
(`titlebar.rs`'s `app-menu-button`, a gpui-kit popup menu) is not one of them. So while a page is shown, the menu
and its submenus open beneath the page wherever they overlap it: View > Find… cannot be seen or reached there.

Found by Task 24 of `docs/plans/20261107-upstream-sync-v0.41.1.md` (Claim v0.41.1 parity): the UI cases
`conformance/tests/ui/test_html_overlay.py::test_find_over_a_text_page_searches_the_page` and
`::test_find_with_a_page_over_the_other_pane_searches_the_terminal` pass on Linux (ratcheted in
`rust-passing.txt`) but fail on Windows ("menu item Find… stays outside its menu after 40 wheel ticks" / "no menu
command View > Find…"), while `test_find_bar_counts_the_matches`, with no page, passes. They stay out of
`conformance/rust-passing-windows.txt` until this is fixed. The cause above is the likely one, not yet confirmed
in the VM.

Likely fix: count an open app menu (the title bar already tracks it, `on_menu_open_change`) and any other popup menu
the window draws (row and tab context menus) as covering in `window.rs`, then confirm on the Windows VM and add the
two cases to `rust-passing-windows.txt`. Check macOS's WKWebView host too, which shares `set_covered`. Not fixed in
the run: it was found in the parity-claim task, needs a Windows GUI loop to confirm, and touches every popup.
