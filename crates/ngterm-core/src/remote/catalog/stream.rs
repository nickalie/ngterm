//! One remote window's catalog as its stream brings it (the frame half of upstream's `RemoteCatalogMirror.swift`): a
//! snapshot, whole or in chunks with their commit, then numbered deltas. A gap in the revisions, another epoch or bad
//! chunks ask for a fresh subscription, whose snapshot replaces the catalog; a revision already seen is dropped. A lost
//! stream keeps the last catalog; a server that cannot serve a remote window is refused for good.

use ngterm_proto::{AppIdentity, CatalogVersions};
use uuid::Uuid;

use crate::catalog::frames::{self, CatalogChunks, CatalogFrame, CatalogSnapshot};

/// Where a remote window's connection is (upstream's `RemoteCatalogMirror.State`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogState {
    /// Subscribing: the rows shown are the last ones until the snapshot arrives.
    Connecting,
    Connected,
    /// The stream was lost and is retried; the rows and screens shown are the last ones.
    Disconnected(String),
    /// Final: the server cannot serve this window, so nothing retries.
    Refused(String),
}

impl CatalogState {
    /// The state's wire name and its reason, as `tree` and `window.list` report them.
    #[must_use]
    pub fn wire(&self) -> (&'static str, Option<&str>) {
        match self {
            Self::Connecting => ("connecting", None),
            Self::Connected => ("connected", None),
            Self::Disconnected(reason) => ("disconnected", Some(reason)),
            Self::Refused(reason) => ("refused", Some(reason)),
        }
    }

    /// What the window says about its connection to `host` (upstream's `windowNotice`), `None` once it is up.
    #[must_use]
    pub fn window_notice(&self, host: &str) -> Option<String> {
        match self {
            Self::Connecting => Some(format!("Connecting to {host}…")),
            Self::Connected => None,
            Self::Disconnected(reason) => Some(format!("Disconnected: {reason}. Retrying.")),
            Self::Refused(reason) => Some(reason.clone()),
        }
    }

    #[must_use]
    pub const fn is_refused(&self) -> bool {
        matches!(self, Self::Refused(_))
    }
}

/// Why `host`, which advertised `versions` (`None`: nothing) and runs `app`, cannot serve a remote window (upstream's
/// `versionRefusal`, its server named `ngterm-headless`); `None` when it can.
#[must_use]
pub fn version_refusal(host: &str, versions: Option<&CatalogVersions>, app: Option<&AppIdentity>) -> Option<String> {
    let Some(versions) = versions else {
        let build = app.map_or_else(|| "an ngterm".to_owned(), |app| format!("ngterm {}", app.version));
        return Some(format!(
            "{host} runs {build} without a catalog, an app with windows rather than ngterm-headless; a remote window \
             needs ngterm-headless"
        ));
    };
    if !(1..=frames::VERSION).contains(&versions.catalog) {
        return Some(format!(
            "{host} speaks catalog {}, this Mac speaks catalog {}",
            versions.catalog,
            frames::VERSION
        ));
    }
    if versions.protocol < 1 {
        return Some(format!(
            "{host} speaks presentation {}, this Mac speaks presentation {}",
            versions.protocol,
            crate::remote::frames::VERSION
        ));
    }
    None
}

/// The refusal of a window whose server's catalog another window already shows.
#[must_use]
pub fn duplicate_refusal(host: &str) -> String {
    format!("another window already shows the catalog of {host}")
}

/// What one frame did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Received {
    /// The frame to send back: an ack for a ping.
    pub reply: Option<CatalogFrame>,
    /// The catalog moved: `Some(true)` for a whole snapshot, `Some(false)` for a revision.
    pub moved: Option<bool>,
}

/// A remote window's catalog and its stream's state.
#[derive(Debug)]
pub struct CatalogStream {
    host: String,
    state: CatalogState,
    catalog: Option<CatalogSnapshot>,
    needs_snapshot: bool,
    chunks: CatalogChunks,
    hello_epoch: Option<String>,
}

impl CatalogStream {
    /// `host`'s stream, connecting, with nothing mirrored yet.
    #[must_use]
    pub fn new(host: &str) -> Self {
        Self {
            host: host.to_owned(),
            state: CatalogState::Connecting,
            catalog: None,
            needs_snapshot: false,
            chunks: CatalogChunks::default(),
            hello_epoch: None,
        }
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub const fn state(&self) -> &CatalogState {
        &self.state
    }

    /// The catalog as last mirrored, kept across a lost stream.
    #[must_use]
    pub const fn catalog(&self) -> Option<&CatalogSnapshot> {
        self.catalog.as_ref()
    }

    /// Whether a gap asks for a fresh subscription.
    #[must_use]
    pub const fn needs_snapshot(&self) -> bool {
        self.needs_snapshot
    }

    /// Judges what the server's `version` advertised before anything subscribes: a refusal ends the retries.
    pub fn check_versions(&mut self, versions: Option<&CatalogVersions>, app: Option<&AppIdentity>) -> bool {
        match version_refusal(&self.host, versions, app) {
            Some(refusal) => {
                self.refuse(refusal);
                false
            }
            None => true,
        }
    }

    /// A new stream: Connecting, with the last rows kept, until its snapshot.
    pub fn subscribing(&mut self) {
        if self.state.is_refused() {
            return;
        }
        self.needs_snapshot = false;
        self.chunks = CatalogChunks::default();
        self.hello_epoch = None;
        self.state = CatalogState::Connecting;
    }

    /// The stream was lost for `reason`: Disconnected, the last rows kept, nothing queued for the server.
    pub fn connection_lost(&mut self, reason: String) {
        if !self.state.is_refused() {
            self.state = CatalogState::Disconnected(reason);
        }
    }

    /// The server cannot serve this window: final.
    pub fn refuse(&mut self, reason: String) {
        self.state = CatalogState::Refused(reason);
    }

    /// Takes one frame; `bind` hears a snapshot's catalog id first and refuses one another window shows.
    pub fn receive(&mut self, frame: CatalogFrame, bind: &mut dyn FnMut(Uuid) -> bool) -> Received {
        let mut received = Received::default();
        if self.state.is_refused() {
            return received;
        }
        match frame {
            CatalogFrame::Hello(hello) => match version_refusal(&self.host, hello.versions.as_ref(), None) {
                Some(refusal) => self.refuse(refusal),
                None => self.hello_epoch = hello.epoch,
            },
            CatalogFrame::Snapshot(snapshot) => received.moved = self.take(snapshot, bind),
            CatalogFrame::Chunk { index, data } => {
                if self.chunks.add(index, &data).is_err() {
                    self.needs_snapshot = true;
                }
            }
            CatalogFrame::Commit { epoch, rev, chunks } => match self.chunks.commit(&epoch, rev, chunks) {
                Ok(snapshot) => received.moved = self.take(snapshot, bind),
                Err(_) => self.needs_snapshot = true,
            },
            CatalogFrame::Delta { epoch, rev, change } => {
                if self.advance(&epoch, rev)
                    && let Some(catalog) = self.catalog.as_mut()
                {
                    catalog.apply(&change);
                    catalog.rev = rev;
                    received.moved = Some(false);
                }
            }
            CatalogFrame::Unknown { epoch, rev, .. } => {
                // a newer server's change this build cannot read moves the cursor past it; the next snapshot heals it
                if let (Some(epoch), Some(rev)) = (epoch, rev)
                    && self.advance(&epoch, rev)
                    && let Some(catalog) = self.catalog.as_mut()
                {
                    catalog.rev = rev;
                    received.moved = Some(false);
                }
            }
            CatalogFrame::Ping => received.reply = Some(CatalogFrame::Ack),
            CatalogFrame::Ack => {}
        }
        received
    }

    /// Whether `(epoch, rev)` is the next revision; another epoch or a gap asks for a snapshot, and a revision already
    /// seen is dropped.
    fn advance(&mut self, epoch: &str, rev: i64) -> bool {
        let Some(catalog) = self.catalog.as_ref().filter(|_| !self.needs_snapshot) else {
            return false;
        };
        if epoch == catalog.epoch && rev == catalog.rev + 1 {
            return true;
        }
        if epoch != catalog.epoch || rev > catalog.rev + 1 {
            self.needs_snapshot = true;
        }
        false
    }

    fn take(&mut self, snapshot: CatalogSnapshot, bind: &mut dyn FnMut(Uuid) -> bool) -> Option<bool> {
        if self.hello_epoch.as_ref().is_some_and(|epoch| *epoch != snapshot.epoch) {
            self.needs_snapshot = true;
            return None;
        }
        if !bind(snapshot.catalog_id) {
            self.refuse(duplicate_refusal(&self.host));
            return None;
        }
        self.catalog = Some(snapshot);
        self.needs_snapshot = false;
        self.state = CatalogState::Connected;
        Some(true)
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
