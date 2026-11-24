//! The catalog a headless origin (`ngterm-headless`) serves to remote windows (upstream's agterm-server catalog,
//! `ServerCatalog.swift` and `ServerStreams.swift`): every workspace and session of its open windows with their live
//! panes, numbered by revision within this run's epoch.
//!
//! - [`frames`]: the stream's wire; [`diff`]: the changes between two catalogs; [`hub`]: the revisions and the
//!   subscribers; [`operations`]: the receipts of structural commands sent with an operation id; [`root`]: the
//!   catalog's durable id;
//! - `catalog.subscribe` hands its connection to a stream once its `ok` is out: the viewer's hello subscribes it, then
//!   it gets the snapshot and every change, published on each commit; [`receipts`] answers an `opID` and `catalog.op`.
//!
//! Only a core told to serve it ([`Core::serve_catalog`]) has a catalog; an app with windows refuses both commands
//! and any `opID` (`commands::catalog`). Sessions attached from another machine are that machine's and stay out.

pub mod diff;
pub mod frames;
pub mod hub;
pub mod operations;
pub(crate) mod receipts;
pub mod root;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use ngterm_proto::ControlLiveEndpoint;
use uuid::Uuid;

use self::frames::{
    CatalogFrame, CatalogPage, CatalogPane, CatalogSession, CatalogSessionFields, CatalogSnapshot, CatalogWorkspace,
    Lifecycle,
};
use self::hub::{CatalogHub, CatalogSink, CloseReason, SubscriberId};
use self::operations::CatalogOperationTable;
use crate::Core;
use crate::LineChannel;
use crate::html::HtmlSource;
use crate::model::{Session, SessionContent};

/// How long a subscriber may leave a ping unanswered (upstream's `CatalogHub(staleTimeout: 30)`).
pub const STALE_TIMEOUT: Duration = Duration::from_secs(30);
/// How often the frontend runs [`Core::catalog_heartbeat`] while a stream is up (upstream's `heartbeatSeconds`).
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
/// How long a stream may stay silent before its hello (upstream's `presentationHelloDeadline`).
pub const HELLO_DEADLINE: Duration = Duration::from_secs(10);
/// The write bound of a stream (upstream's `CatalogStream.limits`).
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// The file in the state directory that keeps the operation table.
pub const OPERATIONS_FILE_NAME: &str = "catalog-operations.json";

/// One adopted `catalog.subscribe` connection.
struct Stream {
    channel: Rc<dyn LineChannel>,
    subscriber: Option<SubscriberId>,
}

/// The hub's sink over a stream's connection: each frame one line.
struct ChannelSink(Rc<dyn LineChannel>);

impl CatalogSink for ChannelSink {
    fn offer(&self, frame: &CatalogFrame) -> bool {
        match frames::encode(frame) {
            Ok(mut line) => {
                // the channel adds the newline itself
                line.pop();
                self.0.send(line)
            }
            Err(error) => {
                tracing::error!(%error, "closing a catalog stream on an unsendable frame");
                false
            }
        }
    }

    fn close(&self, reason: CloseReason) {
        tracing::info!(?reason, "closing a catalog stream");
        self.0.shutdown();
    }
}

/// What a served catalog keeps.
pub(crate) struct Served {
    pub(crate) hub: CatalogHub,
    catalog_id: Uuid,
    pub(crate) table: CatalogOperationTable,
    /// Where the table is written; `None` keeps it in memory only.
    table_path: Option<PathBuf>,
    /// The last write of the table failed: no entry is handed out as a receipt until one lands.
    pub(crate) unsaved: bool,
}

/// The catalog's runtime; [`Self::served`] is `None` on an app with windows.
#[derive(Default)]
pub(crate) struct CatalogRuntime {
    pub(crate) served: Option<Served>,
    streams: BTreeMap<u64, Stream>,
    next_serial: u64,
}

impl std::fmt::Debug for CatalogRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogRuntime")
            .field("served", &self.served.as_ref().map(|served| &served.hub))
            .field("streams", &self.streams.keys().collect::<Vec<_>>())
            .field("next_serial", &self.next_serial)
            .finish()
    }
}

impl Served {
    /// Writes the table; whether it landed. A failed write keeps the table in memory and marks it unsaved.
    pub(crate) fn save(&mut self) -> bool {
        let Some(path) = &self.table_path else {
            self.unsaved = false;
            return true;
        };
        let written = serde_json::to_vec(&self.table)
            .map_err(std::io::Error::other)
            .and_then(|bytes| crate::persist::json::write_atomic(path, &bytes));
        if let Err(error) = &written {
            tracing::error!(%error, path = %path.display(), "the catalog's operation table was not saved");
        }
        self.unsaved = written.is_err();
        !self.unsaved
    }

    /// Whether the table on disk holds what memory does, retrying a write that failed.
    pub(crate) fn durable(&mut self) -> bool {
        if self.unsaved {
            self.save();
        }
        !self.unsaved
    }
}

/// The table `path` holds; a missing, unreadable or malformed one reads as empty and keeps the catalog.
fn load_table(path: &Path) -> CatalogOperationTable {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            tracing::warn!(%error, path = %path.display(), "the catalog's operation table is malformed; starting empty");
            CatalogOperationTable::default()
        }),
        Err(_) => CatalogOperationTable::default(),
    }
}

/// `session`'s catalog entry, its panes reached through `endpoint`.
fn catalog_session(session: &Session, endpoint: &ControlLiveEndpoint) -> CatalogSession {
    let page = (session.content() == SessionContent::Page)
        .then(|| session.overlay.page())
        .flatten()
        .map(|overlay| CatalogPage {
            kind: if matches!(overlay.source, HtmlSource::Url(_)) {
                "url"
            } else {
                "file"
            }
            .to_owned(),
            identity: overlay.identity(),
            endpoint: endpoint.clone(),
        });
    let has_split = session.split.exists && session.split_pane_identity.is_some();
    let panes = if page.is_some() || session.content() == SessionContent::Page {
        Vec::new()
    } else {
        std::iter::once(session.pane_identity)
            .chain(session.split_pane_identity.filter(|_| has_split))
            .map(|identity| CatalogPane {
                identity,
                daemon: crate::live::daemon_name(identity),
                endpoint: endpoint.clone(),
            })
            .collect()
    };
    let fields = CatalogSessionFields {
        id: session.id(),
        name: session.display_name(),
        parent: session.parent_id(),
        cwd: session.effective_cwd().to_owned(),
        primary: if session.content() == SessionContent::Page {
            session.id()
        } else {
            session.pane_identity
        },
        axis: has_split.then(|| session.split.axis.wire_name().to_owned()),
        shown: has_split && session.split.shown,
        lifecycle: Lifecycle::Live,
        scratch: None,
        page,
        custom_name: Some(session.custom_name.clone().unwrap_or_default()),
    };
    CatalogSession::new(fields, panes)
}

impl Core {
    /// Serves the catalog from here on, as `ngterm-headless` does: `catalog_id` is the state directory's
    /// ([`root::load`]), and the operation table lives in `directory` (in memory only without one). An operation
    /// that committed in an earlier run is answered at a revision this run's viewers can reach.
    pub fn serve_catalog(&mut self, catalog_id: Uuid, directory: Option<&Path>) {
        let table_path = directory.map(|directory| directory.join(OPERATIONS_FILE_NAME));
        let mut table = table_path.as_deref().map(load_table).unwrap_or_default();
        let epoch = self.remote.respawns.epoch().to_owned();
        table.adopt(&epoch, 0);
        self.catalog.served = Some(Served {
            hub: CatalogHub::new(epoch, STALE_TIMEOUT, Box::new(Instant::now)),
            catalog_id,
            table,
            table_path,
            unsaved: false,
        });
        self.refresh_catalog();
    }

    /// Whether this core serves a catalog.
    #[must_use]
    pub fn serves_catalog(&self) -> bool {
        self.catalog.served.is_some()
    }

    /// The open windows' workspaces and sessions in order, each live session with its left pane and, while it has a
    /// split, its right one; a page row lists no pane. Epoch and revision are the hub's to stamp.
    pub(crate) fn catalog_source(&self, catalog_id: Uuid) -> CatalogSnapshot {
        let endpoint = self.live.endpoint().cloned().unwrap_or_else(|| ControlLiveEndpoint {
            attach_bridge: None,
            executable: String::new(),
            socket_directory: String::new(),
            peer: None,
        });
        let workspaces = self
            .library
            .open_ids()
            .into_iter()
            .filter_map(|window| self.library.store(window))
            .flat_map(crate::model::AppStore::workspaces)
            .map(|workspace| CatalogWorkspace {
                id: workspace.id(),
                name: workspace.name.clone(),
                sessions: workspace
                    .sessions
                    .iter()
                    .filter(|session| session.remote_host().is_none())
                    .map(|session| catalog_session(session, &endpoint))
                    .collect(),
            })
            .collect();
        CatalogSnapshot {
            epoch: String::new(),
            rev: 0,
            catalog_id,
            versions: self.catalog_versions(),
            workspaces,
        }
    }

    /// Publishes what changed in the catalog since the last commit; nothing without a served catalog.
    pub(crate) fn refresh_catalog(&mut self) {
        let Some(catalog_id) = self.catalog.served.as_ref().map(|served| served.catalog_id) else {
            return;
        };
        let source = self.catalog_source(catalog_id);
        if let Some(served) = self.catalog.served.as_mut() {
            served.hub.refresh(source);
        }
    }

    /// Takes over a connection whose `catalog.subscribe` was answered `ok` (upstream's `adoptCatalogStream`); the
    /// serial the frontend reports its lines, its close and its hello deadline under.
    pub fn adopt_catalog_stream(&mut self, channel: Box<dyn LineChannel>) -> u64 {
        self.catalog.next_serial += 1;
        let serial = self.catalog.next_serial;
        self.catalog.streams.insert(
            serial,
            Stream {
                channel: Rc::from(channel),
                subscriber: None,
            },
        );
        serial
    }

    /// One line from stream `serial` (upstream's `CatalogStream.receive`): the viewer speaks first, its hello
    /// subscribes it, and everything after goes to the hub. A bad frame or a first frame that is not a usable hello
    /// closes the stream.
    pub fn catalog_line(&mut self, serial: u64, line: &[u8]) {
        let Some(stream) = self.catalog.streams.get(&serial) else {
            return;
        };
        let (channel, subscriber) = (Rc::clone(&stream.channel), stream.subscriber);
        let frame = match frames::decode(line) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::warn!(%error, "closing a catalog stream on a bad frame");
                channel.shutdown();
                return;
            }
        };
        let Some(catalog_id) = self.catalog.served.as_ref().map(|served| served.catalog_id) else {
            channel.shutdown();
            return;
        };
        if let Some(subscriber) = subscriber {
            if let Some(served) = self.catalog.served.as_mut() {
                served.hub.receive(&frame, subscriber);
            }
            return;
        }
        let CatalogFrame::Hello(hello) = frame else {
            tracing::warn!("closing a catalog stream that did not open with hello");
            channel.shutdown();
            return;
        };
        let source = self.catalog_source(catalog_id);
        let versions = self.catalog_versions();
        let Some(served) = self.catalog.served.as_mut() else {
            return;
        };
        match served
            .hub
            .subscribe(&hello, Box::new(ChannelSink(Rc::clone(&channel))), versions, source)
        {
            Ok(id) => {
                if let Some(stream) = self.catalog.streams.get_mut(&serial) {
                    stream.subscriber = Some(id);
                }
            }
            Err(error) => {
                tracing::warn!(%error, "refusing a catalog stream");
                channel.shutdown();
            }
        }
    }

    /// Stream `serial` closed: its subscriber leaves the hub.
    pub fn catalog_closed(&mut self, serial: u64) {
        let Some(stream) = self.catalog.streams.remove(&serial) else {
            return;
        };
        if let (Some(subscriber), Some(served)) = (stream.subscriber, self.catalog.served.as_mut()) {
            served.hub.unsubscribe(subscriber);
        }
    }

    /// Stream `serial`'s hello deadline passed: one that sent none is closed.
    pub fn catalog_hello_deadline(&mut self, serial: u64) {
        if let Some(stream) = self.catalog.streams.get(&serial)
            && stream.subscriber.is_none()
        {
            tracing::info!("closing a catalog stream that sent no hello");
            stream.channel.shutdown();
        }
    }

    /// The heartbeat: stale subscribers dropped, the rest pinged; whether any stream is still up, so the frontend
    /// arms the next one.
    pub fn catalog_heartbeat(&mut self) -> bool {
        if let Some(served) = self.catalog.served.as_mut() {
            served.hub.heartbeat();
        }
        !self.catalog.streams.is_empty()
    }

    /// Closes every catalog stream (the quit path).
    pub fn shutdown_catalog_streams(&mut self) {
        for stream in self.catalog.streams.values() {
            stream.channel.shutdown();
        }
    }
}

#[cfg(test)]
pub(crate) mod fixture;
