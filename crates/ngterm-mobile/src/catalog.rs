//! A headless origin's catalog (`ngterm-headless`; upstream's remote window over agterm-server,
//! `RemoteCatalogClient.swift`, `RemoteCatalogMirror.swift` and `RemoteCatalogActions.swift`): the machine's
//! workspaces and sessions in order, mirrored live from `ngtermctl catalog subscribe`, and the structural actions sent
//! under an operation id.
//!
//! - [`Mirror`] takes the stream's frames: a snapshot (whole, or chunks with their commit), then numbered deltas. A gap
//!   in the revisions, another epoch, a bad frame or bad chunks ask for a fresh subscription, whose snapshot replaces
//!   the rows. A lost stream keeps the last rows and reads as [`CatalogState::Disconnected`]; a machine that cannot
//!   serve a catalog reads as [`CatalogState::Refused`], and its sessions stay on the plain listing.
//! - [`rows`] is what the sessions screen lists from a catalog: its workspaces' order and names, each session's name,
//!   ended sessions marked, page rows marked with their kind (a page companion has no row of its own, as in the plain
//!   listing). What the catalog does not carry (agent status, context, window) comes from the machine's last listing.
//! - [`operations`] sends New Session, Rename Session and End Session with an operation id and reads the receipt.
//!
//! Only a machine whose listing says it is headless gets a catalog: any other keeps the listing it has.

pub mod operations;

#[cfg(test)]
pub(crate) mod tests;

use std::time::Duration;

use ngterm_core::catalog::frames::{
    self, CatalogChunks, CatalogFrame, CatalogHello, CatalogSession, CatalogSnapshot, Lifecycle,
};
use ngterm_core::pane_env::uuid_string;
use ngterm_proto::{CatalogVersions, ControlRemoteTree, RemotePeer};

use crate::sessions::RemoteSession;

/// The waits between a lost stream's retries (upstream's `RemoteCatalogClient.retryDelays`); the last one repeats.
pub const RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
];

/// How long the stream may stay quiet (upstream's `silenceLimit`): the server pings every ten seconds, so this much
/// silence means the link is gone without ssh noticing.
pub const SILENCE_LIMIT: Duration = Duration::from_secs(35);

/// The wait before retry number `attempt` (from 0).
#[must_use]
pub fn retry_delay(attempt: usize) -> Duration {
    RETRY_DELAYS[attempt.min(RETRY_DELAYS.len() - 1)]
}

/// Where a machine's catalog stream is.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CatalogState {
    /// Subscribing: the rows shown are the last ones until the snapshot arrives.
    Connecting,
    /// The rows follow the machine live.
    Connected,
    /// The stream was lost and is retried; the rows shown are the last ones.
    Disconnected { reason: String },
    /// The machine cannot serve a catalog, and nothing retries: its sessions stay on the plain listing.
    Refused { reason: String },
}

impl CatalogState {
    /// The state's name, for the log: a reason may quote the machine.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Disconnected { .. } => "disconnected",
            Self::Refused { .. } => "refused",
        }
    }
}

/// Why `host`, which advertised `versions` (`None`: nothing), cannot serve this app a catalog (upstream's
/// `versionRefusal`, which names the Mac); `None` when it can.
#[must_use]
pub fn version_refusal(host: &str, versions: Option<&CatalogVersions>) -> Option<String> {
    let Some(versions) = versions else {
        return Some(format!(
            "{host} runs an ngterm without a catalog; a live session list needs ngterm-headless"
        ));
    };
    if !(1..=frames::VERSION).contains(&versions.catalog) {
        return Some(format!(
            "{host} speaks catalog {}, this app speaks catalog {}",
            versions.catalog,
            frames::VERSION
        ));
    }
    if versions.protocol < 1 {
        return Some(format!(
            "{host} speaks presentation {}, this app speaks presentation {}",
            versions.protocol,
            ngterm_core::remote::frames::VERSION
        ));
    }
    None
}

/// The hello this app opens a stream with: it shows page rows.
#[must_use]
pub fn hello_line() -> Vec<u8> {
    let hello = CatalogFrame::Hello(CatalogHello {
        version: frames::VERSION,
        page_rows: Some(true),
        ..CatalogHello::default()
    });
    frames::encode(&hello).unwrap_or_default()
}

/// What one frame did to the mirror.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Received {
    /// The frame to send back (an ack for a ping).
    pub reply: Option<CatalogFrame>,
    /// The catalog moved: `Some(true)` for a whole snapshot, `Some(false)` for a revision.
    pub moved: Option<bool>,
}

/// One machine's catalog as the stream has brought it (upstream's `RemoteCatalogMirror`, without the window's store).
#[derive(Debug)]
pub struct Mirror {
    host: String,
    state: CatalogState,
    catalog: Option<CatalogSnapshot>,
    needs_snapshot: bool,
    chunks: CatalogChunks,
    hello_epoch: Option<String>,
}

impl Mirror {
    /// A mirror of `host`'s catalog, the name its messages give the machine; nothing mirrored yet.
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

    /// The name its messages give the machine.
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

    /// Whether the stream must be subscribed again for a fresh snapshot.
    #[must_use]
    pub const fn needs_snapshot(&self) -> bool {
        self.needs_snapshot
    }

    #[must_use]
    pub const fn is_refused(&self) -> bool {
        matches!(self.state, CatalogState::Refused { .. })
    }

    /// Whether the catalog is live at `epoch` and has reached revision `rev`.
    #[must_use]
    pub fn reaches(&self, epoch: &str, rev: i64) -> bool {
        self.state == CatalogState::Connected
            && self
                .catalog
                .as_ref()
                .is_some_and(|catalog| catalog.epoch == epoch && catalog.rev >= rev)
    }

    /// A new stream: Connecting, with the last rows kept, until its snapshot.
    pub fn subscribing(&mut self) {
        if self.is_refused() {
            return;
        }
        self.needs_snapshot = false;
        self.chunks = CatalogChunks::default();
        self.hello_epoch = None;
        self.state = CatalogState::Connecting;
    }

    /// The stream was lost for `reason`: Disconnected, with the last rows kept.
    pub fn connection_lost(&mut self, reason: String) {
        if !self.is_refused() {
            self.state = CatalogState::Disconnected { reason };
        }
    }

    /// The machine cannot serve a catalog: the rows go, so its plain listing shows.
    pub fn refuse(&mut self, reason: String) {
        self.catalog = None;
        self.state = CatalogState::Refused { reason };
    }

    /// Drops session `id` and its companions from the rows, as the machine closed them; the stream's own change
    /// later finds them gone.
    pub fn forget(&mut self, id: &str) {
        let Some(catalog) = self.catalog.as_mut() else {
            return;
        };
        for workspace in &mut catalog.workspaces {
            workspace.sessions.retain(|session| {
                let own = uuid_string(session.id());
                let parent = session.fields.parent.map(uuid_string);
                !own.eq_ignore_ascii_case(id) && !parent.is_some_and(|parent| parent.eq_ignore_ascii_case(id))
            });
        }
    }

    /// Takes one frame of the stream.
    pub fn receive(&mut self, frame: CatalogFrame) -> Received {
        let mut received = Received::default();
        if self.is_refused() {
            return received;
        }
        match frame {
            CatalogFrame::Hello(hello) => {
                if let Some(refusal) = version_refusal(&self.host, hello.versions.as_ref()) {
                    self.refuse(refusal);
                } else {
                    self.hello_epoch = hello.epoch;
                }
            }
            CatalogFrame::Snapshot(snapshot) => received.moved = self.take(snapshot),
            CatalogFrame::Chunk { index, data } => {
                if self.chunks.add(index, &data).is_err() {
                    self.needs_snapshot = true;
                }
            }
            CatalogFrame::Commit { epoch, rev, chunks } => match self.chunks.commit(&epoch, rev, chunks) {
                Ok(snapshot) => received.moved = self.take(snapshot),
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
                // a newer server's change this app cannot read moves the cursor past it; the next snapshot heals it
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

    fn take(&mut self, snapshot: CatalogSnapshot) -> Option<bool> {
        if self.hello_epoch.as_ref().is_some_and(|epoch| *epoch != snapshot.epoch) {
            self.needs_snapshot = true;
            return None;
        }
        self.catalog = Some(snapshot);
        self.needs_snapshot = false;
        self.state = CatalogState::Connected;
        Some(true)
    }
}

/// Session `id` of `catalog` (any case).
#[must_use]
pub fn session<'a>(catalog: &'a CatalogSnapshot, id: &str) -> Option<&'a CatalogSession> {
    catalog
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.sessions)
        .find(|session| uuid_string(session.id()).eq_ignore_ascii_case(id))
}

/// Whether a catalog session is a page row.
#[must_use]
pub const fn is_page(session: &CatalogSession) -> bool {
    session.fields.page.is_some()
}

/// Whether a catalog session has ended with nothing left to show: no pane and no scratch.
#[must_use]
pub fn has_ended(session: &CatalogSession) -> bool {
    session.fields.lifecycle == Lifecycle::Ended && session.fields.scratch.is_none()
}

/// The rows the sessions screen lists from `catalog`: each workspace in order, each top-level row followed by its
/// companions, then any companion whose parent is not a top-level row there. A page companion has no row. The status,
/// context and window of a session come from `tree`, the machine's last listing, when it lists the session.
#[must_use]
pub fn rows(catalog: &CatalogSnapshot, tree: Option<&ControlRemoteTree>) -> Vec<RemoteSession> {
    let mut rows = Vec::new();
    for workspace in &catalog.workspaces {
        let listed: Vec<&CatalogSession> = workspace
            .sessions
            .iter()
            .filter(|session| !(is_page(session) && session.fields.parent.is_some()))
            .collect();
        let top: Vec<&CatalogSession> = listed
            .iter()
            .copied()
            .filter(|session| session.fields.parent.is_none())
            .collect();
        let mut placed = Vec::new();
        for parent in &top {
            placed.push(*parent);
            placed.extend(
                listed
                    .iter()
                    .copied()
                    .filter(|session| session.fields.parent == Some(parent.id())),
            );
        }
        let orphans: Vec<&CatalogSession> = listed
            .iter()
            .copied()
            .filter(|session| !placed.iter().any(|kept| kept.id() == session.id()))
            .collect();
        placed.extend(orphans);
        rows.extend(placed.into_iter().map(|session| row(session, &workspace.name, tree)));
    }
    rows
}

fn row(session: &CatalogSession, workspace: &str, tree: Option<&ControlRemoteTree>) -> RemoteSession {
    let id = uuid_string(session.id());
    let listed = tree.and_then(|tree| {
        tree.apps()
            .flat_map(|app| &app.sessions)
            .find(|listed| listed.id.eq_ignore_ascii_case(&id))
    });
    let fields = &session.fields;
    RemoteSession {
        name: fields.name.clone(),
        window_name: listed.map(|listed| listed.window_name.clone()).unwrap_or_default(),
        workspace_name: workspace.to_owned(),
        context: listed.and_then(|listed| listed.context.clone()),
        cwd: fields.cwd.clone(),
        status: listed.and_then(|listed| listed.status.clone()),
        status_color: listed.and_then(|listed| listed.status_color.clone()),
        status_shape: listed.and_then(|listed| listed.status_shape.clone()),
        parent: fields.parent.map(uuid_string),
        pane_count: u32::try_from(session.panes.len()).unwrap_or(u32::MAX),
        split_axis: fields.axis.clone(),
        peer: Some(RemotePeer::Ngterm.into()),
        ended: fields.lifecycle == Lifecycle::Ended,
        page: fields.page.as_ref().map(|page| page.kind.clone()),
        id,
    }
}
