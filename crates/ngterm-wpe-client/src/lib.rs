//! Socket client and shared types for the Linux HTML overlay worker. No `WebKit` libraries are linked here, and no
//! pkg-config build script runs, so the app depends on this crate alone and only the worker needs an engine.
//!
//! - The worker is the sibling `ngterm-web` binary (crate `ngterm-wpe`). [`worker::pin_worker`] opens it as a
//!   descriptor at app startup, so an update that replaces the install cannot swap it; the handshake compares the
//!   protocol and [`worker::build_identity`], and a mismatch asks for a restart.
//! - The app resolves an [`EngineSource`] (`ngterm_update::web_engine`, with [`worker::probe_system`]) and the client
//!   turns it into the worker's command. [`EngineSource::System`] adds nothing and removes `LD_LIBRARY_PATH`.
//!   [`EngineSource::Runtime`] sets `LD_LIBRARY_PATH` (which `ngterm-web` drops once the loader has read it),
//!   `GIO_MODULE_DIR`, `GST_PLUGIN_SYSTEM_PATH_1_0`, `GST_PLUGIN_SCANNER`, `GST_REGISTRY_1_0` and
//!   `WEBKIT_INJECTED_BUNDLE_PATH`, and removes `GIO_EXTRA_MODULES`, `GST_PLUGIN_PATH` and `GST_PLUGIN_PATH_1_0`. All
//!   of this is on the worker's command alone: the app, its panes and its session servers never see it.
//! - A runtime's worker holds a shared lock on its `.in-use` file, taken before the exec, so cleanup by another version
//!   of the app leaves a runtime in use alone.
//! - `AppArmor`: the worker inherits the profile attached to `ngterm`, and needs no entry of its own.

/// `from_raw` and `raw` on the engine's id types: a consumer's stand-in for the engine in its own tests makes ids
/// with them, and logs read them; the engine itself never takes an id it did not hand out.
macro_rules! raw_ids {
    ($($id:ident),+) => {
        $(
            impl $id {
                /// The id numbered `raw` (crate docs: for a consumer's stand-in engine).
                #[must_use]
                pub const fn from_raw(raw: u64) -> Self {
                    Self(raw)
                }

                /// The id's number.
                #[must_use]
                pub const fn raw(self) -> u64 {
                    self.0
                }
            }
        )+
    };
}
pub(crate) use raw_ids;

#[doc(hidden)]
pub mod config;
#[doc(hidden)]
pub mod event;
#[doc(hidden)]
pub mod frames;
#[doc(hidden)]
pub mod host;
mod host_commands;
#[doc(hidden)]
pub mod input;
mod source;
#[doc(hidden)]
pub mod spec;
pub mod worker;
pub use source::{EngineSource, RUNTIME_ENV, RUNTIME_LOCK_ENV};

pub use config::{
    BufferFormat, ConfigError, DmaBufTarget, FrameMode, HostConfig, MODIFIER_INVALID, MODIFIER_LINEAR, fourcc,
};
pub use event::{
    Area, ContextMenu, EditingCommand, Event, LoadError, LoadErrorKind, LoadEvent, MenuToken, NavigationType,
    OptionItem, OptionKind, OptionMenu, PageState, PolicyKind, PolicyRequest, PolicyToken, ReplyToken, SchemeError,
    SchemeRequest, SchemeResponse, SchemeToken, ScriptMessage, Termination,
};
pub use frames::{BufferId, DmaBuf, Frame, FrameContent, Plane, Rect, ShmRegion};
pub use host::{Evaluation, Host, Load, StartError};
pub use input::{Button, Input, Key, Modifiers, Pointer, PointerKind, Scroll};
pub use spec::{
    Geometry, InjectionTime, MessageHandler, Rgba, SessionId, SessionSpec, Settings, UserScript, ViewId, ViewSpec,
};
