---
worth: yes
where: crates/ngterm-gpui/src/html_overlay/view.rs
added: 2026-10-24
---
# A page with a fixed-position native button never paints its content

An HTML overlay whose document holds a native `<button>` with `position: fixed` shows only its first, empty frame
in the GPUI frontend: the engine sends one frame, the element answers it, and no frame with the page's content
follows (`FrameStats { shm: 1, answered: 1 }` on lavapipe, `dmabuf: 1` on Mesa). Found while writing the headless
replica cases of Task 24 of `docs/plans/20261023-upstream-sync-v0.38.1.md`: a plain text-loaded file page
`<body style='background:#c03030'><button style='position:fixed;left:0;top:0;width:200px;height:150px'>go</button>`
in `crates/ngterm-gpui/tests/html_overlay/cases.rs`'s `loads` case reproduces it, while the same page without
`position: fixed`, or a fixed `<div>`, paints. The same page served by the engine harness
(`crates/ngterm-wpe/tests/headless`) paints, so the frames stall between the engine and the GPUI element (a
composited layer the frame pipeline does not wake on is a suspect). Agent-written pages with a fixed toolbar of
buttons would show nothing. The replica case for tagged controls uses a fixed `<div role=button>` meanwhile; the
defect predates that task and was left as it was.
