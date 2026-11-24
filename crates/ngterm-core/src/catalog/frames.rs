//! The catalog stream's wire (upstream's `CatalogFrames.swift`): the catalog's shape, its changes, and one
//! newline-delimited JSON frame per line under the presentation stream's frame cap. An oversized snapshot travels as
//! base64 chunks that apply only at their commit.

use base64::Engine as _;
use ngterm_proto::{CatalogVersions, ControlLiveEndpoint};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::skip_serializing_none;
use uuid::Uuid;

use crate::persist::upper;
pub use crate::remote::frames::FrameError;
use crate::remote::frames::MAX_FRAME_BYTES;

/// The catalog stream's version (upstream's `CatalogCodec.version`).
pub const VERSION: i64 = ngterm_proto::CATALOG_VERSION;
/// Raw snapshot bytes per chunk: base64 grows them by a third and still leaves room under the frame cap.
pub const CHUNK_BYTES: usize = 128 * 1024;

/// `ended` for a session whose panes are all gone without their shell exiting; it stays in the catalog until closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lifecycle {
    Live,
    Ended,
}

/// One daemon-backed pane: its stable identity, the live session carrying it and how to reach it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPane {
    #[serde(serialize_with = "upper::uuid")]
    pub identity: Uuid,
    pub daemon: String,
    pub endpoint: ControlLiveEndpoint,
}

/// A session's scratch on the server: its live session, attached like a pane, and whether it is shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogScratch {
    pub pane: CatalogPane,
    pub shown: bool,
}

/// A page row's page: `file` or `url`, its identity text, and the server's live endpoint, which names the server for
/// the row's binding as a pane's endpoint does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPage {
    pub kind: String,
    pub identity: String,
    pub endpoint: ControlLiveEndpoint,
}

/// A session without its panes, which travel as their own changes.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSessionFields {
    #[serde(serialize_with = "upper::uuid")]
    pub id: Uuid,
    /// The resolved label.
    pub name: String,
    #[serde(default, serialize_with = "upper::option")]
    pub parent: Option<Uuid>,
    pub cwd: String,
    /// The left pane's identity; a page row's own id.
    #[serde(serialize_with = "upper::uuid")]
    pub primary: Uuid,
    /// Present only for a session that has a split.
    #[serde(default)]
    pub axis: Option<String>,
    /// Whether the split is shown.
    pub shown: bool,
    pub lifecycle: Lifecycle,
    /// The session's scratch while its live session runs.
    #[serde(default)]
    pub scratch: Option<CatalogScratch>,
    /// Marks a page row, which has no pane.
    #[serde(default)]
    pub page: Option<CatalogPage>,
    /// The manual name, `""` when there is none; a server predating it omits it.
    #[serde(default)]
    pub custom_name: Option<String>,
}

/// A session with its panes, the primary pane first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "SessionWire")]
pub struct CatalogSession {
    #[serde(flatten)]
    pub fields: CatalogSessionFields,
    pub panes: Vec<CatalogPane>,
}

#[derive(Deserialize)]
struct SessionWire {
    #[serde(flatten)]
    fields: CatalogSessionFields,
    panes: Vec<CatalogPane>,
}

impl From<SessionWire> for CatalogSession {
    fn from(wire: SessionWire) -> Self {
        Self::new(wire.fields, wire.panes)
    }
}

impl CatalogSession {
    #[must_use]
    pub fn new(fields: CatalogSessionFields, panes: Vec<CatalogPane>) -> Self {
        let mut session = Self { fields, panes };
        session.normalize();
        session
    }

    #[must_use]
    pub const fn id(&self) -> Uuid {
        self.fields.id
    }

    /// Puts the primary pane first, the one pane order the wire knows.
    pub fn normalize(&mut self) {
        if let Some(index) = self.panes.iter().position(|pane| pane.identity == self.fields.primary)
            && index > 0
        {
            let primary = self.panes.remove(index);
            self.panes.insert(0, primary);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogWorkspace {
    #[serde(serialize_with = "upper::uuid")]
    pub id: Uuid,
    pub name: String,
    pub sessions: Vec<CatalogSession>,
}

/// The whole catalog at `(epoch, rev)`. A new epoch is a restarted server, whose revisions start over and never
/// compare with an earlier epoch's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogSnapshot {
    pub epoch: String,
    pub rev: i64,
    #[serde(rename = "catalogID", serialize_with = "upper::uuid")]
    pub catalog_id: Uuid,
    pub versions: CatalogVersions,
    pub workspaces: Vec<CatalogWorkspace>,
}

/// One structural change, applied in order by [`CatalogSnapshot::apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogChange {
    WorkspaceAdded {
        workspace: CatalogWorkspace,
        index: i64,
    },
    WorkspaceChanged {
        id: Uuid,
        name: String,
    },
    WorkspaceRemoved(Uuid),
    WorkspaceOrder(Vec<Uuid>),
    SessionAdded {
        session: CatalogSession,
        workspace: Uuid,
        index: i64,
    },
    SessionChanged(CatalogSessionFields),
    SessionRemoved(Uuid),
    /// Puts a session into another workspace; the viewer keeps its row.
    SessionMoved {
        id: Uuid,
        workspace: Uuid,
        index: i64,
    },
    SessionOrder {
        ids: Vec<Uuid>,
        workspace: Uuid,
    },
    PaneAdded {
        pane: CatalogPane,
        session: Uuid,
    },
    PaneChanged {
        pane: CatalogPane,
        session: Uuid,
    },
    PaneRemoved {
        identity: Uuid,
        session: Uuid,
    },
}

impl CatalogChange {
    /// The change's frame kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::WorkspaceAdded { .. } => "workspace.added",
            Self::WorkspaceChanged { .. } => "workspace.changed",
            Self::WorkspaceRemoved(_) => "workspace.removed",
            Self::WorkspaceOrder(_) => "workspace.order",
            Self::SessionAdded { .. } => "session.added",
            Self::SessionChanged(_) => "session.changed",
            Self::SessionRemoved(_) => "session.removed",
            Self::SessionMoved { .. } => "session.moved",
            Self::SessionOrder { .. } => "session.order",
            Self::PaneAdded { .. } => "pane.added",
            Self::PaneChanged { .. } => "pane.changed",
            Self::PaneRemoved { .. } => "pane.removed",
        }
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogHello {
    pub version: i64,
    /// Set on the server's answer, never on the viewer's opening hello.
    #[serde(default)]
    pub epoch: Option<String>,
    #[serde(default)]
    pub versions: Option<CatalogVersions>,
    /// Set on a viewer's opening hello when it shows page rows; an older viewer is sent none.
    #[serde(default)]
    pub page_rows: Option<bool>,
}

/// One line of the catalog stream. A delta's kind is its change's kind; an unknown kind decodes as
/// [`CatalogFrame::Unknown`], keeping the envelope's epoch and revision so a viewer can step past it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "a frame lives only between its line and the change it applies"
)]
pub enum CatalogFrame {
    Hello(CatalogHello),
    Snapshot(CatalogSnapshot),
    /// One base64 slice of an oversized snapshot's JSON; nothing applies until its commit.
    Chunk {
        index: i64,
        data: String,
    },
    Commit {
        epoch: String,
        rev: i64,
        chunks: i64,
    },
    Delta {
        epoch: String,
        rev: i64,
        change: CatalogChange,
    },
    Ping,
    Ack,
    Unknown {
        kind: String,
        epoch: Option<String>,
        rev: Option<i64>,
    },
}

/// A revision with no change for this viewer: one a viewer without page rows is not sent.
pub const SKIP_KIND: &str = "catalog.skip";

fn malformed(error: impl std::fmt::Display) -> FrameError {
    FrameError::Malformed(error.to_string())
}

fn value_of(value: &impl Serialize) -> Result<Value, FrameError> {
    serde_json::to_value(value).map_err(malformed)
}

fn uuid_value(id: Uuid) -> Value {
    Value::String(crate::pane_env::uuid_string(id))
}

fn uuids_value(ids: &[Uuid]) -> Value {
    Value::Array(ids.iter().copied().map(uuid_value).collect())
}

impl CatalogFrame {
    fn object(&self) -> Result<Map<String, Value>, FrameError> {
        let mut map = Map::new();
        let mut put = |key: &str, value: Value| {
            map.insert(key.to_owned(), value);
        };
        match self {
            Self::Hello(hello) => {
                put("kind", "hello".into());
                put("hello", value_of(hello)?);
            }
            Self::Snapshot(snapshot) => {
                put("kind", "snapshot".into());
                put("snapshot", value_of(snapshot)?);
            }
            Self::Chunk { index, data } => {
                put("kind", "snapshot.chunk".into());
                put("index", (*index).into());
                put("data", data.as_str().into());
            }
            Self::Commit { epoch, rev, chunks } => {
                put("kind", "snapshot.commit".into());
                put("epoch", epoch.as_str().into());
                put("rev", (*rev).into());
                put("chunks", (*chunks).into());
            }
            Self::Delta { epoch, rev, change } => {
                put("kind", change.kind().into());
                put("epoch", epoch.as_str().into());
                put("rev", (*rev).into());
                change_fields(change, &mut put)?;
            }
            Self::Ping => put("kind", "ping".into()),
            Self::Ack => put("kind", "ack".into()),
            Self::Unknown { kind, epoch, rev } => {
                put("kind", kind.as_str().into());
                if let Some(epoch) = epoch {
                    put("epoch", epoch.as_str().into());
                }
                if let Some(rev) = rev {
                    put("rev", (*rev).into());
                }
            }
        }
        Ok(map)
    }

    fn from_object(map: &Map<String, Value>) -> Result<Self, FrameError> {
        let kind = field::<String>(map, "kind")?;
        Ok(match kind.as_str() {
            "hello" => Self::Hello(field(map, "hello")?),
            "snapshot" => Self::Snapshot(field(map, "snapshot")?),
            "snapshot.chunk" => Self::Chunk {
                index: field(map, "index")?,
                data: field(map, "data")?,
            },
            "snapshot.commit" => Self::Commit {
                epoch: field(map, "epoch")?,
                rev: field(map, "rev")?,
                chunks: field(map, "chunks")?,
            },
            "ping" => Self::Ping,
            "ack" => Self::Ack,
            other => match change_of(other, map)? {
                Some(change) => Self::Delta {
                    epoch: field(map, "epoch")?,
                    rev: field(map, "rev")?,
                    change,
                },
                None => Self::Unknown {
                    kind,
                    epoch: optional(map, "epoch")?,
                    rev: optional(map, "rev")?,
                },
            },
        })
    }
}

fn change_fields(change: &CatalogChange, put: &mut impl FnMut(&str, Value)) -> Result<(), FrameError> {
    match change {
        CatalogChange::WorkspaceAdded { workspace, index } => {
            put("workspace", value_of(workspace)?);
            put("index", (*index).into());
        }
        CatalogChange::WorkspaceChanged { id, name } => {
            put("id", uuid_value(*id));
            put("name", name.as_str().into());
        }
        CatalogChange::WorkspaceRemoved(id) | CatalogChange::SessionRemoved(id) => put("id", uuid_value(*id)),
        CatalogChange::WorkspaceOrder(ids) => put("ids", uuids_value(ids)),
        CatalogChange::SessionAdded {
            session,
            workspace,
            index,
        } => {
            put("session", value_of(session)?);
            put("workspace", uuid_value(*workspace));
            put("index", (*index).into());
        }
        CatalogChange::SessionChanged(fields) => put("session", value_of(fields)?),
        CatalogChange::SessionMoved { id, workspace, index } => {
            put("id", uuid_value(*id));
            put("workspace", uuid_value(*workspace));
            put("index", (*index).into());
        }
        CatalogChange::SessionOrder { ids, workspace } => {
            put("ids", uuids_value(ids));
            put("workspace", uuid_value(*workspace));
        }
        CatalogChange::PaneAdded { pane, session } | CatalogChange::PaneChanged { pane, session } => {
            put("pane", value_of(pane)?);
            put("session", uuid_value(*session));
        }
        CatalogChange::PaneRemoved { identity, session } => {
            put("identity", uuid_value(*identity));
            put("session", uuid_value(*session));
        }
    }
    Ok(())
}

fn change_of(kind: &str, map: &Map<String, Value>) -> Result<Option<CatalogChange>, FrameError> {
    Ok(Some(match kind {
        "workspace.added" => CatalogChange::WorkspaceAdded {
            workspace: field(map, "workspace")?,
            index: field(map, "index")?,
        },
        "workspace.changed" => CatalogChange::WorkspaceChanged {
            id: field(map, "id")?,
            name: field(map, "name")?,
        },
        "workspace.removed" => CatalogChange::WorkspaceRemoved(field(map, "id")?),
        "workspace.order" => CatalogChange::WorkspaceOrder(field(map, "ids")?),
        "session.added" => CatalogChange::SessionAdded {
            session: field(map, "session")?,
            workspace: field(map, "workspace")?,
            index: field(map, "index")?,
        },
        "session.changed" => CatalogChange::SessionChanged(field(map, "session")?),
        "session.removed" => CatalogChange::SessionRemoved(field(map, "id")?),
        "session.moved" => CatalogChange::SessionMoved {
            id: field(map, "id")?,
            workspace: field(map, "workspace")?,
            index: field(map, "index")?,
        },
        "session.order" => CatalogChange::SessionOrder {
            ids: field(map, "ids")?,
            workspace: field(map, "workspace")?,
        },
        "pane.added" => CatalogChange::PaneAdded {
            pane: field(map, "pane")?,
            session: field(map, "session")?,
        },
        "pane.changed" => CatalogChange::PaneChanged {
            pane: field(map, "pane")?,
            session: field(map, "session")?,
        },
        "pane.removed" => CatalogChange::PaneRemoved {
            identity: field(map, "identity")?,
            session: field(map, "session")?,
        },
        _ => return Ok(None),
    }))
}

fn field<T: serde::de::DeserializeOwned>(map: &Map<String, Value>, key: &str) -> Result<T, FrameError> {
    let value = map.get(key).ok_or_else(|| malformed(format!("no {key}")))?;
    T::deserialize(value).map_err(|error| malformed(format!("{key}: {error}")))
}

fn optional<T: serde::de::DeserializeOwned>(map: &Map<String, Value>, key: &str) -> Result<Option<T>, FrameError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => field(map, key).map(Some),
    }
}

/// Encodes `frame` as one line, its newline included.
///
/// # Errors
///
/// The frame cannot be encoded, or its line would reach the frame cap.
pub fn encode(frame: &CatalogFrame) -> Result<Vec<u8>, FrameError> {
    let mut line = serde_json::to_vec(&Value::Object(frame.object()?)).map_err(malformed)?;
    if line.len() >= MAX_FRAME_BYTES {
        return Err(FrameError::Oversize(line.len()));
    }
    line.push(b'\n');
    Ok(line)
}

/// Decodes one line, its newline excluded.
///
/// # Errors
///
/// The line is over the frame cap, or not a frame.
pub fn decode(line: &[u8]) -> Result<CatalogFrame, FrameError> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Oversize(line.len()));
    }
    let value: Value = serde_json::from_slice(line).map_err(malformed)?;
    let Value::Object(map) = value else {
        return Err(malformed("not an object"));
    };
    CatalogFrame::from_object(&map)
}

/// The snapshot as one frame, or as chunks and a commit when one frame would pass the cap.
///
/// # Errors
///
/// The snapshot cannot be encoded.
pub fn snapshot_frames(snapshot: &CatalogSnapshot) -> Result<Vec<CatalogFrame>, FrameError> {
    let whole = CatalogFrame::Snapshot(snapshot.clone());
    if encode(&whole).is_ok() {
        return Ok(vec![whole]);
    }
    let data = serde_json::to_vec(snapshot).map_err(malformed)?;
    let mut frames: Vec<CatalogFrame> = data
        .chunks(CHUNK_BYTES)
        .zip(0_i64..)
        .map(|(slice, index)| CatalogFrame::Chunk {
            index,
            data: base64::engine::general_purpose::STANDARD.encode(slice),
        })
        .collect();
    let chunks = i64::try_from(frames.len()).unwrap_or(i64::MAX);
    frames.push(CatalogFrame::Commit {
        epoch: snapshot.epoch.clone(),
        rev: snapshot.rev,
        chunks,
    });
    Ok(frames)
}

/// Why gathered chunks did not make a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkError {
    OutOfOrder(i64),
    Incomplete { expected: i64, got: i64 },
    Mismatched,
    Malformed(String),
}

/// Gathers an oversized snapshot's chunks and yields it only at a commit that matches them.
#[derive(Debug, Default)]
pub struct CatalogChunks {
    data: Vec<u8>,
    count: i64,
}

impl CatalogChunks {
    /// Takes chunk `index`.
    ///
    /// # Errors
    ///
    /// The chunk is not the next one or not base64; what was gathered is dropped.
    pub fn add(&mut self, index: i64, slice: &str) -> Result<(), ChunkError> {
        let bytes = base64::engine::general_purpose::STANDARD.decode(slice);
        match bytes {
            Ok(bytes) if index == self.count => {
                self.data.extend_from_slice(&bytes);
                self.count += 1;
                Ok(())
            }
            _ => {
                self.reset();
                Err(ChunkError::OutOfOrder(index))
            }
        }
    }

    /// The gathered snapshot, which must be at `(epoch, rev)` and in `chunks` chunks; either way the chunks are gone.
    ///
    /// # Errors
    ///
    /// Too few or too many chunks, a snapshot of another position, or bytes that are no snapshot.
    pub fn commit(&mut self, epoch: &str, rev: i64, chunks: i64) -> Result<CatalogSnapshot, ChunkError> {
        let data = std::mem::take(&mut self.data);
        let count = std::mem::take(&mut self.count);
        if chunks != count {
            return Err(ChunkError::Incomplete {
                expected: chunks,
                got: count,
            });
        }
        let snapshot: CatalogSnapshot =
            serde_json::from_slice(&data).map_err(|error| ChunkError::Malformed(error.to_string()))?;
        if snapshot.epoch != epoch || snapshot.rev != rev {
            return Err(ChunkError::Mismatched);
        }
        Ok(snapshot)
    }

    fn reset(&mut self) {
        self.data.clear();
        self.count = 0;
    }
}

#[cfg(test)]
#[path = "frames_tests.rs"]
mod tests;
