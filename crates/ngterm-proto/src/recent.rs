//! The `recent.*` payloads (upstream's `ControlRecent.swift`): a Recent Closed entry as `recent.list` reports it, and
//! what `recent.open` reopened and where.

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// One Recent Closed entry (upstream's `ControlRecentItem`): a summary, never the snapshot it restores from.
/// `workspace_id` is the parent workspace for a session entry and the workspace itself for a workspace entry;
/// `session_ids` are the sessions a reopen brings back. `closed_at` is Unix seconds.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlRecentItem {
    pub id: String,
    /// `session` or `workspace`.
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    pub closed_at: f64,
    #[serde(rename = "workspaceID")]
    pub workspace_id: String,
    #[serde(rename = "sessionIDs")]
    pub session_ids: Vec<String>,
}

/// What `recent.open` did (upstream's `ControlRecentOpen`). `window` is where the entry was restored, which can differ
/// from the requested window when another window already holds its session or workspace. `complete` is false when the
/// entry is still in the list afterwards, so a later open can finish the restore.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlRecentOpen {
    pub item: ControlRecentItem,
    pub window: String,
    pub complete: bool,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::ControlResult;

    fn item(subtitle: Option<&str>) -> ControlRecentItem {
        ControlRecentItem {
            id: "R1".to_owned(),
            kind: "session".to_owned(),
            title: "api".to_owned(),
            subtitle: subtitle.map(str::to_owned),
            closed_at: 1_700_000_000.5,
            workspace_id: "W1".to_owned(),
            session_ids: vec!["S1".to_owned()],
        }
    }

    #[test]
    fn an_item_encodes_upstreams_keys_in_order_and_drops_a_missing_subtitle() {
        assert_eq!(
            serde_json::to_string(&item(Some("project"))).unwrap(),
            r#"{"id":"R1","kind":"session","title":"api","subtitle":"project","closedAt":1700000000.5,"workspaceID":"W1","sessionIDs":["S1"]}"#
        );
        assert_eq!(
            serde_json::to_value(item(None)).unwrap(),
            json!({"id": "R1", "kind": "session", "title": "api", "closedAt": 1_700_000_000.5,
                   "workspaceID": "W1", "sessionIDs": ["S1"]})
        );
    }

    /// `payloadsRoundTripThroughTheWire` (`ControlRecentTests.swift`).
    #[test]
    fn payloads_round_trip_through_the_wire() {
        let result = ControlResult {
            recent: Some(vec![item(Some("project"))]),
            recent_open: Some(ControlRecentOpen {
                item: item(None),
                window: "W1".to_owned(),
                complete: false,
            }),
            ..ControlResult::default()
        };
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(encoded.find("\"recent\"").unwrap() < encoded.find("\"recentOpen\"").unwrap());
        assert_eq!(serde_json::from_str::<ControlResult>(&encoded).unwrap(), result);
    }
}
