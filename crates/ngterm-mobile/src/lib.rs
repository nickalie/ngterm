//! `ngterm-mobile`: the Rust core of ngterm's Android app, a remote-only viewer of ngterm machines (`ngterm-headless`
//! on a server, or a desktop instance) over ssh. It speaks the protocol desktop's File ▸ Attach Remote speaks:
//! `ngtermctl live tree` for a machine's sessions, `ngtermctl live present SESSION` for a session's presentation,
//! `ngterm-session attach` per shown pane, and the control lines that create and end sessions. It runs no local
//! sessions, so it builds `ngterm-term` without `local`.
//!
//! Kotlin reaches it through `UniFFI` (proc-macro form); `scripts/build-android.sh` builds the `cdylib` and generates
//! the bindings with the crate's own `uniffi-bindgen` binary (feature `bindgen`).
//!
//! Module map: [`api`] (the facade, its records, events and callback interfaces), [`actor`] (the thread, its runtime
//! and commands), [`transport`] (the ssh seam, its channels, the russh transport and the scripted fake behind the
//! `fakes` feature), [`keys`] (generating, importing and loading the app's ssh keys), [`sessions`] (a machine's session
//! list, New Session and End Session), [`presentation`] (attached sessions' presentation streams and what they
//! show), [`pane`] (shown panes: their attach, terminal, leadership, ends and clipboard reads), [`reconnect`] (keeping
//! machines connected: lost connections, network changes, suspend and resume), [`rows`] (the packed row buffer a pane
//! is drawn from), [`chat`] (the agent chat view: what each pane runs, the chat channels of the panes shown as a chat,
//! their older pages and the agents' slash-command catalogs; the screen matchers and the actions that take the lead and
//! answer the agent in its pane, [`chat::input`]).
//!
//! Threading model:
//! - One [`actor`] thread per [`MobileCore`] runs a current-thread tokio runtime and a `LocalSet`. It owns the
//!   transport, one connection per machine with its exec channels, the `!Send` presentation clients and their
//!   one-second tick, so none of them is `Send` or locked.
//! - The [`api`] objects are thin `Send + Sync` handles: a [`MobileCore`] call posts a command to the actor and awaits
//!   its answer (a Kotlin `suspend fun`). A [`Pane`] handle's input goes to its terminal directly, on the caller's
//!   thread, so typing never waits on the actor.
//! - Each shown pane has a feeder thread (its channel into the terminal's `StreamFeed`, which blocks while the terminal
//!   is behind and so holds the channel back) and a writer thread (the terminal's `StreamOut`: input, replies and
//!   resizes, down the channel). Both report back to the actor.
//! - Events reach the app through the [`EventSink`] on the actor's thread, never the caller's.
//!
//! Seams: the app implements three `UniFFI` callback interfaces, [`EventSink`] (every [`MobileEvent`]), [`KeyVault`]
//! (the Keystore-wrapped private keys, unwrapped per connection and wiped after use) and [`HostKeyStore`] (host keys
//! pinned on first use). Notifications, asks and the clipboard are events the app acts on, answered back through
//! [`MobileCore`]. Inside the crate, [`transport::Transport`] is the seam between the actor and ssh: russh in the app,
//! the scripted fake (feature `fakes`) in tests.

uniffi::setup_scaffolding!();

pub mod actor;
pub mod api;
pub mod chat;
pub mod keys;
pub mod pane;
pub mod presentation;
pub mod reconnect;
pub mod rows;
pub mod sessions;
pub mod transport;

pub use api::{
    CellPoint, EventSink, HostKeyStore, KeyVault, Machine, MachineState, MobileCore, MobileError, MobileEvent, Pane,
    PaneColors, PaneInputError, PaneKey, PaneKeyAction, PaneMouse, PaneMouseAction, PaneMouseButton, PaneSendOutcome,
    PaneSize, PaneWheel, PaneWheelOutcome, VaultError,
};
pub use chat::input::{AgentInputOutcome, QuestionAnswer};
pub use chat::{
    AgentCommand, AgentCommandKind, ChatApproval, ChatBody, ChatEvent, ChatNoticeKind, ChatQuestion, ChatQuestionItem,
    ChatQuestionOption, ChatState, ChatTool, ChatToolKind, ChatToolStatus, filter_commands,
};
pub use keys::{KeyError, KeyPair};
pub use pane::{ClipboardReply, PaneClipboard, PaneEnd, PaneRef, PaneRole};
pub use presentation::{
    AskReply, PresentationRole, RemoteAsk, RemoteAskButton, RemoteAskChoice, RemoteAskChoices, RemoteAskInput,
    RemoteHud, RemoteLayout, RemoteNotice, RemoteStatus, StreamState,
};
pub use sessions::RemoteSession;
