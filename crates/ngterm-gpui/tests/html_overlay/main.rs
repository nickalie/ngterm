//! HTML pages end to end, headless: the overlay's registry starts the real WPE `WebKit` engine on the GPU GPUI's
//! headless renderer opened, a page is mounted and loaded, and its frames come back through the external texture
//! element into GPUI's renderer, read back: dmabufs on a GPU Mesa drives (Intel, AMD), shared memory on lavapipe, which
//! has no render node for the engine to render on. No display, no compositor, no session bus.
//!
//! `WebKit` and GPUI's headless renderer run once per process, so this harness (no libtest) runs each GPU in a child
//! process of its own, with an environment built from scratch (no `NGTERM_*`/`AGTERM_*`/`ZMX_*`, private XDG
//! directories): a plain process, as the app is, whose registry starts the engine in a worker where WPE `WebKit` is
//! vendored (`ngterm_wpe::worker`, the reason `main` calls `serve_if_asked` first). Where a piece is missing (lavapipe,
//! a Mesa render node) its child is skipped with a message; under `NGTERM_REQUIRE_DISPLAY_TESTS` a missing lavapipe
//! fails.
//!
//! Arguments: a substring selects cases by name; `--list` lists them.
//!
//! Linux only: WPE `WebKit` and the GPUs it renders on. The Windows host's (`WebView2`'s) frontend-neutral parts are the
//! crate's unit tests there.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    linux::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {}
