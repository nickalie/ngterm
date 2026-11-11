//! The terminal pane core, toolkit-neutral, the same on Linux, macOS, Windows and Android: a libghostty-vt terminal
//! fed by an IO thread per pane, its effects as [`PaneEvent`]s for the frontend's main thread, and its screen as
//! [`Frame`]s in this crate's own types.
//!
//! A [`Pane`] has one of four backends, which differ only in where the program's bytes come from and go to:
//!
//! - **PTY**: `Pane::spawn` starts a `PaneCommand` on a PTY (`ngterm-pty`: openpty, or Windows' pseudoconsole), whose
//!   actor moves the bytes; `Pane::launch` runs a plain pane's command or login shell this way.
//! - **Live**: `Pane::launch` attaches a live pane to its session on the session server in process (`ngterm-session`),
//!   with no PTY.
//! - **Painted**: `Pane::launch` runs a HUD's painter in process, whose thread writes the terminal.
//! - **Stream**: [`Pane::stream`] makes a pane whose program runs elsewhere, fed by its embedder's channel (a
//!   remote-only frontend's ssh channel running `ngterm-session attach`): the embedder hands the program's output to a
//!   [`StreamFeed`] and sends what a [`StreamOut`] gives it ([`Outgoing`]) to the channel.
//!
//! The `local` feature (on by default) brings the first three: the PTY (`ngterm-pty`), the live pane's in-process
//! attach (`ngterm-session`), the launch (`Launch`, `Host`, `PaneCommand`, with `ngterm-host`'s shell) and the HUD's
//! painter. Without it the crate holds stream panes alone, with no PTY, session server or process table in its graph:
//! a remote-only frontend's (Android, where `ngterm-session` does not build). Everything else is the same for every
//! kind of pane:
//!
//! - Writing and sizing: [`Pane::write`] queues input, [`Pane::resize`] resizes the VT and the program's end (the
//!   PTY, the session or the channel), [`Pane::frame`] captures the screen for painting, [`Pane::take_events`] drains
//!   the effects.
//! - Input: [`Pane::key`] (a toolkit-neutral [`KeyInput`], encoded for the terminal's keyboard modes),
//!   [`Pane::send`] (a binding's text), [`Pane::mouse`], [`Pane::wheel`] (routed as Ghostty routes the wheel),
//!   [`Pane::focus`], [`Pane::paste`], [`Pane::scroll`].
//! - Selection and read-back: [`Pane::select`], [`Pane::select_word`],
//!   [`Pane::select_line`], [`Pane::select_all`], [`Pane::selection_text`], [`Pane::read_text`],
//!   [`Pane::cursor_column`]; a frame's rows carry their selected columns. With the `screen-read` feature,
//!   `Pane::active_screen` reads the program's screen itself (the active area, whatever the viewport shows) with
//!   the screen generation that `Pane::key_if` and `Pane::paste_if` send under, so a key chosen from it reaches
//!   only that screen.
//! - The pointer: [`Pane::gesture`] runs Ghostty's selection gesture (click counts, drags, autoscroll),
//!   [`Pane::cell_at`] and [`Pane::hyperlink`] say what lies under it.
//! - Search: [`Pane::search`], [`Pane::navigate_search`], [`Pane::search_state`], with
//!   [`PaneEvent::SearchTotal`] and [`PaneEvent::SearchSelected`] as libghostty reported them.
//! - Kitty graphics: every [`Frame`] carries a [`KittyView`] of its images and placements.
//! - A launched pane runs under a `Host` (the terminal identity and shell integration libghostty gave a surface), with
//!   the [`ExitPolicy`] of a launched pane; [`Pane::foreground_pid`] is the terminal's foreground process group (on
//!   Windows the process that stands in for it, from the program's job).
//! - Threads and locks: Technical Details > Threads of the GPUI plan; the details are in the `pane` and `effects`
//!   modules' docs. The IO thread holds the VT lock for one bounded slice of `vt_write` at a time; painting only ever
//!   `try_lock`s it.
//! - The engine boundary: no libghostty-vt type appears in this crate's public API, so the VT engine can be swapped
//!   without touching the frontend.
//!
//! [`init`] installs libghostty-vt's process-global hooks (the PNG decoder for Kitty graphics, the log sink into
//! `tracing`); every pane's constructor calls it, so a frontend need not.

mod actions;
#[cfg(feature = "screen-read")]
mod active;
#[cfg(test)]
#[cfg(feature = "local")]
mod bench;
mod effects;
mod exit;
mod gesture;
mod input;
mod kitty;
#[cfg(feature = "local")]
mod launch;
mod mailbox;
mod outlet;
#[cfg(feature = "local")]
mod painted;
mod pane;
mod placeholder;
mod png;
#[cfg(feature = "local")]
mod pty;
pub mod pwd;
mod render;
mod search;
mod selection;
#[cfg(feature = "local")]
mod session_io;
mod stream;
mod text;
mod wheel;

pub use actions::{FormatRegion, SelectionAdjust, TextFormat};
#[cfg(feature = "screen-read")]
pub use active::ActiveScreen;
/// Native clipboard write completion statuses from the terminal protocol.
pub use ngterm_ghostty_vt::ClipboardWriteResult;

pub use effects::{
    ClipboardAnswer, ClipboardContent, ClipboardLocation, ClipboardPermissionScope, ClipboardReadRequest,
    ClipboardWrite, ColorScheme, ExitStatus, PaneEvent, ProgramNeed, ProgramState, ProgramStatus, Progress,
    TerminalColors, WriteCompletion,
};
pub use exit::{ABNORMAL_EXIT_RUNTIME, ExitPolicy};
pub use gesture::{Autoscroll, CLICK_REPEAT_INTERVAL, Gesture, GestureEvent};
pub use input::{
    KeyInput, ModSides, MouseAction, MouseButton, MouseInput, Padding, Paste, PasteSource, Scroll, Typing,
};
pub use kitty::{
    KittyFormat, KittyImage, KittyImageKey, KittyLayer, KittyPlacement, KittyView, PlacementRect, SourceRect,
};
#[cfg(feature = "local")]
pub use launch::{COMMAND_SHELL, FALLBACK_TERM, Host, Launch, Prepared, resolve_resources, resource_candidates};
pub use pane::{
    DEFAULT_KITTY_IMAGE_LIMIT, DEFAULT_WRITE_LIMIT, FrameSnapshot, GridSize, Pane, PaneConfig, PaneStats, Wake,
};
pub use placeholder::PLACEHOLDER;
#[cfg(feature = "local")]
pub use pty::PaneCommand;
pub use render::{
    Cell, CellFlags, CellWidth, Colors, Cursor, CursorDefaults, CursorPosition, CursorStyle, Frame, MouseShape, Rgb,
    Row, ThemeColors, Underline,
};
pub use search::{Highlight, SearchState, SearchStep};
pub use selection::{GridPoint, PointSpace, SelectionRange, SelectionText};
pub use stream::{Outgoing, StreamFeed, StreamOut};
pub use text::TextRegion;
pub use vt::ReadScope;
pub use wheel::{Wheel, WheelInput};

use ngterm_ghostty_vt as vt;

/// Why a pane operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PaneError {
    /// The PTY could not be opened, or the program could not be started.
    #[error("cannot start the program: {0}")]
    Spawn(String),
    /// A thread of the pane, or its PTY's actor, could not start.
    #[error("pty: {0}")]
    Pty(String),
    /// libghostty-vt refused the operation.
    #[error("terminal: {0}")]
    Terminal(String),
    /// The write queue is full: the child is not reading its input.
    #[error("the pane's input queue is full")]
    Backpressure,
    /// The PTY ended or refused a write: the child side is gone.
    #[error("the pane's pty is closed")]
    Closed,
}

impl PaneError {
    pub(crate) fn terminal(err: vt::Error) -> Self {
        Self::Terminal(err.to_string())
    }
}

/// The scrollback limits (`None`: unlimited); the first reached wins, both at page granularity (~400 KB).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbackLimits {
    pub max_bytes: Option<u64>,
    pub max_lines: Option<u64>,
}

impl Default for ScrollbackLimits {
    /// Ghostty's defaults (`scrollback-limit-bytes = 50000000`, no line limit), as the config parser has them.
    fn default() -> Self {
        ngterm_core::ghostty_config::Scrollback::default().into()
    }
}

impl From<ngterm_core::ghostty_config::Scrollback> for ScrollbackLimits {
    fn from(scrollback: ngterm_core::ghostty_config::Scrollback) -> Self {
        Self {
            max_bytes: scrollback.limit_bytes,
            max_lines: scrollback.limit_lines,
        }
    }
}

impl ScrollbackLimits {
    pub(crate) fn engine(self) -> vt::ScrollbackLimits {
        let size = |limit: Option<u64>| limit.map(|limit| usize::try_from(limit).unwrap_or(usize::MAX));
        vt::ScrollbackLimits {
            max_bytes: size(self.max_bytes),
            max_lines: size(self.max_lines),
        }
    }
}

/// Install libghostty-vt's process-global hooks once: the PNG decoder (Kitty graphics) and the log sink (`tracing`,
/// target `ngterm_term::ghostty`).
pub fn init() {
    vt::sys::install(vt::sys::Hooks {
        png: Some(png::decode),
        log: Some(log),
    });
}

fn log(level: vt::sys::LogLevel, scope: &str, message: &str) {
    match level {
        vt::sys::LogLevel::Error => tracing::error!(target: "ngterm_term::ghostty", scope, "{message}"),
        vt::sys::LogLevel::Warning => tracing::warn!(target: "ngterm_term::ghostty", scope, "{message}"),
        vt::sys::LogLevel::Info => tracing::info!(target: "ngterm_term::ghostty", scope, "{message}"),
        vt::sys::LogLevel::Debug => tracing::debug!(target: "ngterm_term::ghostty", scope, "{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrollback_limits_follow_the_config_and_saturate_into_the_library() {
        let limits = ScrollbackLimits::default();
        assert_eq!(limits.max_bytes, Some(50_000_000));
        assert_eq!(limits.max_lines, None);
        let library = ScrollbackLimits {
            max_bytes: Some(u64::MAX),
            max_lines: Some(10),
        }
        .engine();
        assert_eq!(library.max_bytes, Some(usize::MAX));
        assert_eq!(library.max_lines, Some(10));
    }
}
