//! Socket client and shared types for the Linux HTML overlay worker. No `WebKit` libraries are linked here.

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
#[doc(hidden)]
pub mod spec;
pub mod worker;

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
