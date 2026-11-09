//! WPE `WebKit` host for the GPUI frontend's HTML overlays: pages rendered offscreen by WPE `WebKit` (the
//! `WPEPlatform` API) on an engine thread of their own, frames handed to the consumer as dmabufs on its GPU, or as SHM
//! pixels where dmabuf import fails.
//!
//! - [`Host::start`] spawns the engine thread with its own `GMainContext` (thread-default before any `WebKit` object
//!   exists) and a display that renders on the consumer's render node with the formats it imports ([`HostConfig`],
//!   [`FrameMode`]). One engine per process: `WebKit` binds its main run loop to the first thread that uses it, and
//!   that thread never ends (a shutdown closes everything and leaves it idle; the process then ends through
//!   [`exit_process`], since `WebKit`'s exit handlers must not run off its main thread).
//! - Commands go in through [`Host`]'s methods; everything else comes out as [`Event`]s on one channel: frames, load
//!   progress, what a page shows, and the questions `WebKit` asks (navigation and response policy, script messages,
//!   reads of the consumer's URI schemes), each held open until the consumer answers it by token.
//! - Frames follow `WebKit`'s buffer protocol (module `frames`): a frame counts as rendered once the consumer presented
//!   it, and a dmabuf goes back one frame later; acknowledgements run from a source of their own on the engine's
//!   context, never inside `WebKit`'s own hand-over. A page whose dmabufs the consumer cannot import falls back to
//!   SHM frames while it runs ([`Host::fall_back_to_shm`]).
//! - The `WebKit` surface the overlay uses: browser stores ([`SessionSpec`]: in memory, or on disk for saved logins;
//!   downloads refused), [`Settings`], user scripts and script message handlers, navigation and response policy, URI
//!   schemes, and what a page may not do (new windows, dialogs, file choosers, permissions, full screen) refused.
//! - [`worker`]: the engine in a process of its own, this crate's `ngterm-web` binary. The frontend runs only that: it
//!   uses `ngterm-wpe-client` and never links these engine libraries. A vendored WPE `WebKit` (its helpers at a
//!   compiled-in path, reachable only inside an overlay of `/usr/lib`, [`launcher`]) makes the worker re-execute itself
//!   under that overlay, so the consumer never runs in its user namespace; [`Host::start_worker`] asks for a worker
//!   anywhere. The worker's environment, which its web processes inherit, carries a request size for their `PulseAudio`
//!   streams that keeps Web Audio playing on a Bluetooth sink (module `worker`).
//! - Where the engine comes from: the system's, which `ngterm-web --probe` reports by printing the version it loaded (a
//!   loader failure means none: the binary has no RUNPATH into the app's own libraries), or a downloaded runtime, whose
//!   relocated library finds its helpers beside itself. `ngterm-wpe-client`'s `EngineSource` holds the choice and
//!   scopes a runtime's paths to the worker. `ngterm-web` drops `LD_LIBRARY_PATH` first thing, so `WebKit`'s helpers
//!   and the host's `bwrap` and `xdg-dbus-proxy` never inherit it.
//! - `AppArmor`: `ngterm-web` has no profile of its own. It runs under the one `ngterm` was attached to, as the `bwrap`
//!   it starts does, so the app's sandbox probe reflects the worker's situation.
//!
//! - What `WebKitGTK` draws itself and an offscreen page cannot (module `engine::interaction`): the clipboard
//!   (`WPEPlatform`'s is the engine's own: a page's copy is reported, the desktop's text is handed in before a paste),
//!   `<select>` popups and the context menu (reported for the consumer to draw, answered by token or by editing
//!   command), and input methods (an `WPEInputMethodContext` of the engine's that takes the consumer's composition and
//!   commits and reports the caret's area).
//!
//! Not here: accessibility (an offscreen page has no AT-SPI bridge into the consumer's tree).

mod config;
mod engine;
mod event;
mod frames;
mod host;
mod input;
pub mod launcher;
mod spec;
pub mod worker;

use ngterm_wpe_javascriptcore_sys as jsc_sys;
use ngterm_wpe_platform_sys as platform_sys;
use ngterm_wpe_sys as webkit_sys;

pub use config::{
    BufferFormat, ConfigError, DmaBufTarget, FrameMode, HostConfig, MODIFIER_INVALID, MODIFIER_LINEAR, fourcc,
};
pub use event::{
    Area, ContextMenu, EditingCommand, Event, LoadError, LoadErrorKind, LoadEvent, MenuToken, NavigationType,
    OptionItem, OptionKind, OptionMenu, PageState, PolicyKind, PolicyRequest, PolicyToken, ReplyToken, SchemeError,
    SchemeRequest, SchemeResponse, SchemeToken, ScriptMessage, Termination,
};
pub use frames::{BufferId, DmaBuf, Frame, FrameContent, Plane, Rect, ShmRegion};
pub use host::{Backend, Evaluation, Host, Load, StartError, exit_process};
pub use input::{Button, Input, Key, Modifiers, Pointer, PointerKind, Scroll};
pub use spec::{
    Geometry, InjectionTime, MessageHandler, Rgba, SessionId, SessionSpec, Settings, UserScript, ViewId, ViewSpec,
};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
