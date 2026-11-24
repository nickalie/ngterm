//! The server's record of recent operations (upstream's `CatalogOperations.swift`): each structural command sent with
//! an operation id leaves its receipt here, so a repeat or a lookup is answered from it and the command never runs
//! twice. Bounded: the oldest entries fall off, and where the catalog stood then is kept.

use ngterm_proto::CatalogReceipt;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// A receipt's outcome on the wire.
pub mod outcome {
    pub const APPLIED: &str = "applied";
    pub const NOOP: &str = "noop";
    pub const REFUSED: &str = "refused";
    pub const FENCED: &str = "fenced";
    /// A lookup's miss on a table that may have dropped the id's entry: fenced against a late arrival, but whether it
    /// ran before is lost.
    pub const UNKNOWN: &str = "unknown";
}

/// A receipt of `outcome` for `op_id` in `epoch`.
#[must_use]
pub fn receipt(op_id: &str, epoch: &str, outcome: &str) -> CatalogReceipt {
    CatalogReceipt {
        op_id: op_id.to_owned(),
        epoch: epoch.to_owned(),
        outcome: outcome.to_owned(),
        rev: None,
        error: None,
    }
}

/// A point in a catalog's history: an epoch and a revision within it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPosition {
    pub epoch: String,
    pub rev: i64,
}

impl CatalogPosition {
    #[must_use]
    pub fn new(epoch: &str, rev: i64) -> Self {
        Self {
            epoch: epoch.to_owned(),
            rev,
        }
    }
}

/// The recent operations, saved beside the catalog so one that committed before a restart is still answered after it.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogOperationTable {
    entries: Vec<CatalogReceipt>,
    /// Where the catalog stood when the table last dropped entries.
    #[serde(default)]
    last_drop: Option<CatalogPosition>,
}

impl CatalogOperationTable {
    pub const CAPACITY: usize = 512;

    #[must_use]
    pub fn entries(&self) -> &[CatalogReceipt] {
        &self.entries
    }

    #[must_use]
    pub const fn last_drop(&self) -> Option<&CatalogPosition> {
        self.last_drop.as_ref()
    }

    /// The newest receipt of `op_id`.
    #[must_use]
    pub fn receipt(&self, op_id: &str) -> Option<&CatalogReceipt> {
        self.entries.iter().rev().find(|entry| entry.op_id == op_id)
    }

    /// Keeps `receipt` as its id's newest entry, the catalog standing at `position`.
    pub fn record(&mut self, receipt: CatalogReceipt, position: &CatalogPosition) {
        self.entries.retain(|entry| entry.op_id != receipt.op_id);
        self.entries.push(receipt);
        if self.entries.len() > Self::CAPACITY {
            let over = self.entries.len() - Self::CAPACITY;
            self.entries.drain(..over);
            self.last_drop = Some(position.clone());
        }
    }

    /// Whether an operation sent at `sent` could have had its entry dropped: a drop at or after it in that epoch, or a
    /// drop in any other, since epochs are unordered. `None` counts any drop.
    #[must_use]
    pub fn may_have_dropped(&self, sent: Option<&CatalogPosition>) -> bool {
        let Some(drop) = &self.last_drop else {
            return false;
        };
        sent.is_none_or(|sent| drop.epoch != sent.epoch || drop.rev >= sent.rev)
    }

    /// Moves every applied entry of an earlier epoch to `epoch` at `rev`, a revision a viewer reaches with that epoch's
    /// first snapshot; an old epoch's revisions compare with nothing in a new one.
    pub fn adopt(&mut self, epoch: &str, rev: i64) {
        for entry in &mut self.entries {
            if entry.outcome == outcome::APPLIED && entry.epoch != epoch {
                epoch.clone_into(&mut entry.epoch);
                entry.rev = Some(rev);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn here() -> CatalogPosition {
        CatalogPosition::new("e1", 0)
    }

    fn applied_at(op_id: &str, epoch: &str, rev: Option<i64>) -> CatalogReceipt {
        CatalogReceipt {
            rev,
            ..receipt(op_id, epoch, outcome::APPLIED)
        }
    }

    /// Upstream's `aRecordedIdIsAnsweredWithItsNewestReceipt`.
    #[test]
    fn a_recorded_id_is_answered_with_its_newest_receipt() {
        let mut table = CatalogOperationTable::default();
        table.record(receipt("a", "e1", outcome::APPLIED), &here());
        table.record(receipt("b", "e1", outcome::NOOP), &here());
        table.record(applied_at("a", "e1", Some(4)), &here());
        assert_eq!(table.receipt("a"), Some(&applied_at("a", "e1", Some(4))));
        let ids: Vec<&str> = table.entries().iter().map(|entry| entry.op_id.as_str()).collect();
        assert_eq!(ids, ["b", "a"]);
        assert_eq!(table.receipt("c"), None);
    }

    /// Upstream's `theTableKeepsOnlyTheNewestEntries`.
    #[test]
    fn the_table_keeps_only_the_newest_entries() {
        let mut table = CatalogOperationTable::default();
        for index in 0..CatalogOperationTable::CAPACITY + 3 {
            table.record(receipt(&format!("op{index}"), "e1", outcome::NOOP), &here());
        }
        assert_eq!(table.entries().len(), CatalogOperationTable::CAPACITY);
        assert!(table.receipt("op0").is_none());
        assert!(table.receipt("op2").is_none());
        assert!(table.receipt("op3").is_some());
    }

    /// Upstream's `aDropMarksWhereTheCatalogStood`.
    #[test]
    fn a_drop_marks_where_the_catalog_stood() {
        let mut table = CatalogOperationTable::default();
        for index in 0..CatalogOperationTable::CAPACITY {
            table.record(
                receipt(&format!("op{index}"), "e1", outcome::NOOP),
                &CatalogPosition::new("e1", 1),
            );
        }
        assert_eq!(table.last_drop(), None);
        table.record(receipt("over", "e1", outcome::NOOP), &CatalogPosition::new("e1", 7));
        assert_eq!(table.last_drop(), Some(&CatalogPosition::new("e1", 7)));
    }

    /// Upstream's `aDropCoversAnOperationSentAtOrBeforeIt`.
    #[test]
    fn a_drop_covers_an_operation_sent_at_or_before_it() {
        let position = |epoch, rev| Some(CatalogPosition::new(epoch, rev));
        let cases = [
            (None, position("e1", 3), false),
            (position("e1", 7), position("e1", 8), false),
            (position("e1", 7), position("e1", 7), true),
            (position("e1", 7), position("e1", 2), true),
            (position("e2", 0), position("e1", 9), true),
            (position("e1", 7), None, true),
        ];
        for (drop, sent, covered) in cases {
            let mut table = CatalogOperationTable::default();
            if let Some(drop) = &drop {
                for index in 0..=CatalogOperationTable::CAPACITY {
                    table.record(receipt(&format!("op{index}"), &drop.epoch, outcome::NOOP), drop);
                }
            }
            assert_eq!(table.may_have_dropped(sent.as_ref()), covered, "{drop:?} {sent:?}");
        }
    }

    /// Upstream's `adoptingMovesOnlyAnEarlierEpochsAppliedEntries`.
    #[test]
    fn adopting_moves_only_an_earlier_epochs_applied_entries() {
        let mut table = CatalogOperationTable::default();
        table.record(applied_at("done", "old", Some(9)), &here());
        table.record(applied_at("marker", "old", None), &here());
        table.record(applied_at("same", "new", Some(2)), &here());
        table.record(receipt("noop", "old", outcome::NOOP), &here());
        table.record(receipt("fence", "old", outcome::FENCED), &here());
        table.adopt("new", 0);
        assert_eq!(table.receipt("done"), Some(&applied_at("done", "new", Some(0))));
        assert_eq!(table.receipt("marker"), Some(&applied_at("marker", "new", Some(0))));
        assert_eq!(table.receipt("same").and_then(|entry| entry.rev), Some(2));
        assert_eq!(table.receipt("noop").map(|entry| entry.epoch.as_str()), Some("old"));
        assert_eq!(table.receipt("fence").map(|entry| entry.epoch.as_str()), Some("old"));
    }

    #[test]
    fn the_table_round_trips_through_json() {
        let mut table = CatalogOperationTable::default();
        for index in 0..=CatalogOperationTable::CAPACITY {
            table.record(
                applied_at(&format!("op{index}"), "e1", Some(3)),
                &CatalogPosition::new("e1", 5),
            );
        }
        let json = serde_json::to_string(&table).unwrap();
        assert!(json.contains(r#""lastDrop":{"epoch":"e1","rev":5}"#), "{json}");
        assert_eq!(serde_json::from_str::<CatalogOperationTable>(&json).unwrap(), table);
    }
}
