//! `ngterm-mobile`: the Rust core of ngterm's Android app, a remote-only viewer of ngterm machines (`ngterm-headless`
//! on a server, or a desktop instance) over ssh. It speaks the protocol desktop's File ▸ Attach Remote speaks:
//! `ngtermctl live tree` for a machine's sessions, `ngtermctl live present SESSION` for a session's presentation,
//! `ngterm-session attach` per shown pane, and the control lines that create and end sessions. It runs no local
//! sessions, so it builds `ngterm-term` without `local`.
//!
//! Kotlin reaches it through `UniFFI` (proc-macro form): [`api`] is the whole surface, thin handles that post commands
//! to the core's [`actor`] thread, which owns the [`transport`]'s connections.
//!
//! Module map: [`api`] (the facade, its records, events and callback interfaces), [`actor`] (the thread, its runtime
//! and commands), [`transport`] (the ssh seam, its channels, the russh transport and the scripted fake behind the
//! `fakes` feature), [`keys`] (generating, importing and loading the app's ssh keys), [`sessions`] (a machine's session
//! list, New Session and End Session), [`presentation`] (attached sessions' presentation streams and what they
//! show).

uniffi::setup_scaffolding!();

pub mod actor;
pub mod api;
pub mod keys;
pub mod presentation;
pub mod sessions;
pub mod transport;

pub use api::{
    EventSink, HostKeyStore, KeyVault, Machine, MachineState, MobileCore, MobileError, MobileEvent, Pane, VaultError,
};
pub use keys::{KeyError, KeyPair};
pub use presentation::{
    AskReply, PresentationRole, RemoteAsk, RemoteAskButton, RemoteAskChoice, RemoteAskChoices, RemoteAskInput,
    RemoteHud, RemoteLayout, RemoteNotice, RemoteStatus, StreamState,
};
pub use sessions::RemoteSession;
