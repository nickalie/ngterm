//! `ngterm-mobile`: the mobile apps' Rust core (ngterm's Android and iOS apps), a remote-only viewer of ngterm machines
//! (`ngterm-headless` on a server, or a desktop instance) over ssh. It speaks the protocol desktop's File ▸ Attach
//! Remote speaks: `ngtermctl live tree` for a machine's sessions, `ngtermctl live present SESSION` for a session's
//! presentation, `ngterm-session attach` per shown pane, and the control lines that create and end sessions. It runs
//! no local sessions, so it builds `ngterm-term` without `local`, and with `screen-read`: the program's own screen and
//! the keys sent only to the screen read, which the chat's actions answer an agent with (no desktop binary carries
//! them). For a pane running Claude Code or Codex it also runs the origin's Rust-only `ngtermctl agent chat`,
//! `agent commands` and `agent upload`.
//!
//! The apps reach it through `UniFFI` (proc-macro form), each through its own bindings, which the crate's own
//! `uniffi-bindgen` binary (feature `bindgen`) generates in library mode (`uniffi.toml` configures both):
//! - Kotlin for Android: `scripts/build-android.sh` builds the `cdylib` the app loads and generates the bindings over
//!   it.
//! - Swift for iOS: `scripts/build-ios.sh` builds a `staticlib` per iOS target (`aarch64-apple-ios`,
//!   `aarch64-apple-ios-sim`), giving the crate type on the command line, and generates the bindings over the host
//!   build of the library. The crate holds no iOS-specific code.
//!
//! Module map: [`api`] (the facade, its records, events and callback interfaces), [`actor`] (the thread, its runtime
//! and commands), [`transport`] (the ssh seam, its channels, the russh transport and the scripted fake behind the
//! `fakes` feature), [`keys`] (generating, importing and loading the app's ssh keys), [`sessions`] (a machine's session
//! list, New Session and End Session), [`presentation`] (attached sessions' presentation streams and what they
//! show), [`pane`] (shown panes: their attach, terminal, leadership, ends and clipboard reads), [`reconnect`] (keeping
//! machines connected: lost connections, network changes, suspend and resume), [`rows`] (the packed row buffer a pane
//! is drawn from), [`release`] (verifying a GitHub release's checksums for the Android app's self-update), [`chat`] (the agent chat view: what each pane runs, the chat channels of the panes shown as a chat,
//! their older pages and the agents' slash-command catalogs; the screen matchers and the actions that take the lead and
//! answer the agent in its pane, [`chat::input`]).
//!
//! Threading model:
//! - One [`actor`] thread per [`MobileCore`] runs a current-thread tokio runtime and a `LocalSet`. It owns the
//!   transport, one connection per machine with its exec channels, the `!Send` presentation clients and their
//!   one-second tick, so none of them is `Send` or locked.
//! - The [`api`] objects are thin `Send + Sync` handles: a [`MobileCore`] call posts a command to the actor and awaits
//!   its answer (a Kotlin `suspend fun`, a Swift `async` function). A [`Pane`] handle's input goes to its terminal
//!   directly, on the caller's thread, so typing never waits on the actor.
//! - Each shown pane has a feeder thread (its channel into the terminal's `StreamFeed`, which blocks while the terminal
//!   is behind and so holds the channel back) and a writer thread (the terminal's `StreamOut`: input, replies and
//!   resizes, down the channel). Both report back to the actor.
//! - Events reach the app through the [`EventSink`] on the actor's thread, never the caller's.
//!
//! Seams: the app implements three `UniFFI` callback interfaces, [`EventSink`] (every [`MobileEvent`]), [`KeyVault`]
//! (the private keys the platform keeps, wrapped by the Android Keystore or held in the iOS Keychain, handed over per
//! connection and wiped after use) and [`HostKeyStore`] (host keys pinned on first use). Notifications, asks and the
//! clipboard are events the app acts on, answered back through [`MobileCore`]. Inside the crate,
//! [`transport::Transport`] is the seam between the actor and ssh: russh in the app, the scripted fake (feature
//! `fakes`) in tests.

uniffi::setup_scaffolding!();

pub mod actor;
pub mod api;
pub mod chat;
pub mod keys;
pub mod pane;
pub mod presentation;
pub mod reconnect;
pub mod release;
pub mod rows;
pub mod sessions;
pub mod transport;

pub use api::{
    CellPoint, EventSink, HostKeyStore, KeyVault, Machine, MachineState, MobileCore, MobileError, MobileEvent, Pane,
    PaneColors, PaneInputError, PaneKey, PaneKeyAction, PaneMouse, PaneMouseAction, PaneMouseButton, PaneSendOutcome,
    PaneSize, PaneWheel, PaneWheelOutcome, UploadProgress, VaultError,
};
pub use chat::input::{AgentInputOutcome, QuestionAnswer};
pub use chat::{
    AgentCommand, AgentCommandKind, ChatApproval, ChatBody, ChatEvent, ChatNoticeKind, ChatQuestion, ChatQuestionItem,
    ChatQuestionOption, ChatState, ChatTool, ChatToolKind, ChatToolStatus, ChatUsage, UploadCancel, filter_commands,
};
pub use keys::{KeyError, KeyPair};
pub use pane::{ClipboardPart, ClipboardReply, ClipboardWriteStatus, PaneClipboard, PaneEnd, PaneRef, PaneRole};
pub use presentation::{
    AskReply, PresentationRole, RemoteAsk, RemoteAskButton, RemoteAskChoice, RemoteAskChoices, RemoteAskInput,
    RemoteHud, RemoteLayout, RemoteNotice, RemoteStatus, StreamState,
};
pub use release::{ReleaseError, release_checksum};
pub use sessions::RemoteSession;
