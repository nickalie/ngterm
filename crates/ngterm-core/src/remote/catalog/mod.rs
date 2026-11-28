//! Remote windows (upstream's `RemoteCatalogClient.swift`, `RemoteCatalogMirror.swift`, `RemoteWindowViewState.swift`
//! and `ControlServer+RemoteCatalog.swift`): a window that shows one `ngterm-headless` server's catalog, its rows the
//! server's sessions, each pane an attach to the server's pane.
//!
//! - [`stream`] takes the catalog stream's frames and judges the server's versions.
//! - [`run`] connects one window: it asks the server's `version` first, so a server that cannot serve a catalog is
//!   refused with a message rather than retried, then carries the stream into the window's rows, subscribing again on
//!   a gap and backing off after a lost link. The window's rows follow the catalog through
//!   [`crate::model::CatalogRowContext`].
//! - [`view_state`] is what the window keeps on this machine about how it shows the rows, and [`detach`] hides rows
//!   there while they run on the server.
//! - [`operation`] and [`request`] are the structural actions the window sends its server, and the control requests
//!   read as them; `actions` keeps each one marked until its receipt settles it, and `route` sends them for a control
//!   request or the window's own controls.
//!
//! The stream rides the presentation transport ([`super::PresentationTransport`]): its link key names the window
//! where a row's names the session, so [`crate::Core::viewer_line`] routes each line to its owner.

pub(crate) mod actions;
pub mod detach;
mod gestures;
pub mod operation;
pub(crate) mod request;
mod route;
pub(crate) mod run;
pub mod stream;
pub mod view_state;

use std::time::Duration;

pub use self::actions::RemoteOperationMark;
pub use self::detach::{CloseScope, DetachedItem, DetachedKind};
pub use self::operation::{RemoteOperation, RemoteOperationTarget, RemoteSessionPlacement, RemoteShellOptions};
pub use self::stream::{CatalogState, CatalogStream, version_refusal};
pub use self::view_state::{RemoteWindowViewState, SessionView};

/// The waits between a lost stream's retries (upstream's `RemoteCatalogClient.retryDelays`); the last one repeats.
pub const RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
];

/// How long a stream may stay quiet (upstream's `silenceLimit`): the server pings every ten seconds, so this much
/// silence means the link is gone without ssh noticing.
pub const SILENCE_LIMIT: Duration = Duration::from_secs(35);

/// How long the `version` probe may take (upstream's runner deadline).
pub const PROBE_DEADLINE: Duration = Duration::from_secs(15);

/// The wait before retry number `attempt` (from 0).
#[must_use]
pub fn retry_delay(attempt: usize) -> Duration {
    RETRY_DELAYS[attempt.min(RETRY_DELAYS.len() - 1)]
}
