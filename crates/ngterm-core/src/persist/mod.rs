//! Persistence in upstream's on-disk formats: the window index `windows.json`, each
//! window's `windows/<ID>.json` snapshot, the legacy single-window `workspaces.json`, `settings.json` and Open Recent's
//! `recent-closed.json`, all in the state directory.
//!
//! - [`json`]: Foundation's pretty, key-sorted JSON and its atomic write.
//! - [`snapshot`]: the snapshot and index types, with upstream's lossy decoding.
//! - [`settings`]: the app settings and their store.
//! - [`library`]: the window library's persistence: the launch bootstrap (index, orphan recovery, legacy migration,
//!   seeding), loading a closed window, and the saves the model asks for, the debounced ones through the main loop.
//! - [`recent_closed`]: Open Recent's `recent-closed.json`.

pub mod json;
pub mod library;
pub mod recent_closed;
pub mod settings;
pub mod snapshot;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;

pub use library::{Commit, DebouncedSave, LibraryPersistence, SAVE_DEBOUNCE, arm_debounced_saves};
pub use recent_closed::{
    RECENT_CLOSED_FILE_NAME, RECENT_CLOSED_LIMIT, RecentClosedItem, RecentClosedKind, RecentClosedSession,
    RecentClosedState, RecentClosedStore, RecentClosedWorkspace,
};
pub use settings::{AppSettings, RestoreMode, SettingsStore};
pub use snapshot::{
    PageSlotSnapshot, PageSnapshot, PageSourceSnapshot, PersistenceStore, RemoteSessionSnapshot, SessionSnapshot,
    Snapshot, WindowEntry, WindowsIndex, WorkspaceSnapshot,
};

/// Decodes an optional field LOSSILY, as upstream's `(try? c.decodeIfPresent(...)) ?? nil`: a present value that does
/// not decode as `T` (a wrong type, a malformed id, an unknown enum name) reads as `None` instead of failing the
/// whole document. Pair it with `#[serde(default)]` so a missing key reads as `None` too.
///
/// The value is taken as raw JSON text and decoded on its own, so one that even `serde_json::Value` rejects (a number
/// past `f64`'s range, such as `1e400`) drops this field alone too. A document with lossy fields must therefore be
/// decoded by `serde_json` from its text (`from_str`, `from_slice`, `from_reader`), not from a `Value`.
pub(crate) fn lossy<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let raw = Box::<RawValue>::deserialize(deserializer)?;
    Ok(decode_raw(&raw))
}

/// Decodes a raw JSON value as `T`, `None` when it does not decode.
pub(crate) fn decode_raw<T: DeserializeOwned>(raw: &RawValue) -> Option<T> {
    serde_json::from_str(raw.get()).ok()
}

/// Ids as Foundation encodes a `UUID`: its upper-case `uuidString`.
pub(crate) mod upper {
    use serde::Serializer;
    use uuid::Uuid;

    use crate::pane_env::uuid_string;

    pub(crate) fn uuid<S: Serializer>(id: &Uuid, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&uuid_string(*id))
    }

    #[expect(clippy::ref_option, reason = "serde's serialize_with passes the field by reference")]
    pub(crate) fn option<S: Serializer>(id: &Option<Uuid>, serializer: S) -> Result<S::Ok, S::Error> {
        match id {
            Some(id) => serializer.serialize_str(&uuid_string(*id)),
            None => serializer.serialize_none(),
        }
    }

    #[expect(clippy::ref_option, reason = "serde's serialize_with passes the field by reference")]
    pub(crate) fn option_vec<S: Serializer>(ids: &Option<Vec<Uuid>>, serializer: S) -> Result<S::Ok, S::Error> {
        match ids {
            Some(ids) => serializer.collect_seq(ids.iter().map(|id| uuid_string(*id))),
            None => serializer.serialize_none(),
        }
    }
}

/// Parses a window file's stem as `UUID(uuidString:)` does: the 36-character hyphenated form, either case.
#[must_use]
pub fn parse_uuid_string(text: &str) -> Option<uuid::Uuid> {
    if text.len() != 36 {
        return None;
    }
    uuid::Uuid::try_parse(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize)]
    struct NullableIds {
        #[serde(serialize_with = "upper::option")]
        selected: Option<uuid::Uuid>,
        #[serde(serialize_with = "upper::option_vec")]
        order: Option<Vec<uuid::Uuid>>,
    }

    #[test]
    fn optional_ids_write_null_or_uppercase_strings_in_order() {
        let empty = NullableIds {
            selected: None,
            order: None,
        };
        assert_eq!(
            serde_json::to_value(empty).unwrap(),
            serde_json::json!({"selected": null, "order": null})
        );
        let id = uuid::Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let ids = NullableIds {
            selected: Some(id),
            order: Some(vec![id, uuid::Uuid::nil()]),
        };
        assert_eq!(
            serde_json::to_value(ids).unwrap(),
            serde_json::json!({
                "selected": "6BA7B810-9DAD-11D1-80B4-00C04FD430C8",
                "order": ["6BA7B810-9DAD-11D1-80B4-00C04FD430C8", "00000000-0000-0000-0000-000000000000"]
            })
        );
    }
}
