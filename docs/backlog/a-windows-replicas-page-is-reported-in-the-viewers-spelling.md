---
worth: yes
where: crates/ngterm-gpui/src/html_overlay/policy.rs:456
added: 2026-11-23
---
# A Windows origin's replica page is reported in the viewer's path spelling

On a Linux or macOS viewer, `policy::page_url` cannot make a `file:` URL from a Windows origin's path. It reports the
path of the page address instead: `C:\docs\x.md` becomes `/C:/docs/x.md`, a share `\\server\share\x.md` becomes
`/server/share/x.md`, and a verbatim path becomes `/?/C:/...`. That string is the viewer's `current.page`. The viewer
also sends it back to the origin as `PageShown.address` (`remote::replica_page`), and the origin keeps it as its own
`overlay.current.page`, which tree and `session.overlay` readers see. On the Windows machine this path does not exist,
and the share form names a different path (`C:\server\share\...`).

Plain replica pages had this spelling before the Markdown link pages (task #868) used it. Reviewer's suggested fix: let
`page_info` carry the origin's path as the origin spells it for every `HtmlSource::Replica` page (the grant's path or
`replica_link_path`), not a round trip through a host `file:` URL, and assert that spelling in
`a_windows_replica_link_page_reports_its_file_on_any_host`. It changes the page info of all replica pages on all three
engines, so it is larger than a review fix. Found in the revmux review of task #868, round 2.
