//! What a remote window keeps on this machine about how its server's sessions are shown (upstream's
//! `RemoteWindowViewState.swift`): the selection, the collapsed workspaces, and each row's focus, font size and divider.
//! The sessions themselves are the server's and are never saved here; the state is saved per window, in
//! `remote/<id>.view.json` under the state directory.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{AppStore, Session};
use crate::persist::upper;

/// One row's view (upstream's `SessionView`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    /// The pane identity that had the focus, the split's or the primary's.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "upper::option")]
    pub focused_pane: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_ratio: Option<f64>,
}

impl SessionView {
    /// What `session` shows now.
    #[must_use]
    pub fn of(session: &Session) -> Self {
        let focused = session.split.exists && session.split.focused;
        Self {
            focused_pane: focused.then_some(session.split_pane_identity).flatten(),
            font_size: session.font_size,
            split_ratio: session.split.ratio,
        }
    }

    fn is_empty(&self) -> bool {
        self.focused_pane.is_none() && self.font_size.is_none() && self.split_ratio.is_none()
    }

    /// Restores this view on `session`; a focused pane the row no longer has is ignored.
    pub fn apply(&self, session: &mut Session) {
        session.font_size = self.font_size;
        session.split.ratio = self.split_ratio;
        if let Some(focused) = self.focused_pane
            && session.split.exists
        {
            session.split.focused = session.split_pane_identity == Some(focused);
        }
    }
}

/// A remote window's view state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteWindowViewState {
    #[serde(
        default,
        rename = "selectedSessionID",
        skip_serializing_if = "Option::is_none",
        serialize_with = "upper::option"
    )]
    pub selected_session_id: Option<Uuid>,
    #[serde(default, serialize_with = "upper::slice")]
    pub collapsed_workspaces: Vec<Uuid>,
    /// Written as Swift's `JSONEncoder` writes a `[UUID: SessionView]`: one array of alternating ids and views.
    #[serde(default, with = "pairs")]
    pub sessions: BTreeMap<Uuid, SessionView>,
}

impl RemoteWindowViewState {
    /// Reads `path`; a missing or unreadable file is the default state.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Writes this state to `path`.
    ///
    /// # Errors
    ///
    /// The write failed; the last state on disk stays.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        crate::persist::json::save_json(path, self)
    }

    /// The view state of `store`'s rows (upstream's `capture`); a row whose view is the default is left out.
    #[must_use]
    pub fn capture(store: &AppStore) -> Self {
        let mut state = Self {
            selected_session_id: store.selected_session_id(),
            collapsed_workspaces: store
                .workspaces()
                .iter()
                .filter(|workspace| !workspace.is_expanded)
                .map(crate::model::Workspace::id)
                .collect(),
            sessions: BTreeMap::new(),
        };
        for session in store.sessions() {
            let view = SessionView::of(session);
            if !view.is_empty() {
                state.sessions.insert(session.id(), view);
            }
        }
        state
    }

    /// Restores the view of a row the mirror just created.
    pub fn apply(&self, session: &mut Session) {
        if let Some(view) = self.sessions.get(&session.id()) {
            view.apply(session);
        }
    }
}

/// The alternating `[id, view, id, view, …]` array of upstream's file; an object keyed by id reads too, as earlier
/// builds wrote it.
mod pairs {
    use std::collections::BTreeMap;

    use serde::de::Error as _;
    use serde::ser::SerializeSeq;
    use serde::{Deserialize, Deserializer, Serializer};
    use serde_json::Value;
    use uuid::Uuid;

    use super::SessionView;
    use crate::pane_env::uuid_string;

    pub(super) fn serialize<S: Serializer>(
        map: &BTreeMap<Uuid, SessionView>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(map.len() * 2))?;
        for (id, view) in map {
            seq.serialize_element(&uuid_string(*id))?;
            seq.serialize_element(view)?;
        }
        seq.end()
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Shape {
        Pairs(Vec<Value>),
        Keyed(BTreeMap<Uuid, SessionView>),
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<Uuid, SessionView>, D::Error> {
        let items = match Shape::deserialize(deserializer)? {
            Shape::Keyed(map) => return Ok(map),
            Shape::Pairs(items) => items,
        };
        if items.len() % 2 != 0 {
            return Err(D::Error::custom("an id without a view"));
        }
        items
            .chunks(2)
            .map(|pair| {
                let id = Uuid::deserialize(&pair[0]).map_err(D::Error::custom)?;
                let view = SessionView::deserialize(&pair[1]).map_err(D::Error::custom)?;
                Ok((id, view))
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "view_state_tests.rs"]
mod tests;
