//! The rows a pick shows for the search text, as desktop's picker ranks them (`ngterm-gpui`'s `picker::filter_rows`),
//! so both apps filter alike.

use ngterm_core::fuzzy::fuzzy_rank;
use ngterm_core::palette::{palette_search_keys, pick_custom_row_label};

use super::records::RemotePickItem;

/// One row a picker shows.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PickRow {
    /// The item at `index`, in the origin's order.
    Item { index: u32 },
    /// The search text as the answer (`Use "<query>"`), offered when it matches no item and the pick allows one.
    Custom { label: String, query: String },
}

/// The rows `query` shows of `items`: every item in the origin's order for a blank query (trimmed of whitespace and
/// newlines), else the items whose label matches, best first (a subtitle is never searched); with `allow_custom`, a
/// query matching none offers the custom row alone.
#[uniffi::export]
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "UniFFI passes the foreign side's arguments owned"
)]
pub fn pick_rows(items: Vec<RemotePickItem>, query: String, allow_custom: bool) -> Vec<PickRow> {
    let query = query.trim();
    let index = |at: usize| PickRow::Item {
        index: u32::try_from(at).unwrap_or(u32::MAX),
    };
    if query.is_empty() {
        return (0..items.len()).map(index).collect();
    }
    let ranked = fuzzy_rank(query, 0..items.len(), |at| {
        items
            .get(*at)
            .map(|item| palette_search_keys(&item.label, item.subtitle.as_deref(), true))
            .unwrap_or_default()
    });
    if let Some(label) = pick_custom_row_label(query, ranked.len(), allow_custom) {
        return vec![PickRow::Custom {
            label,
            query: query.to_owned(),
        }];
    }
    ranked.into_iter().map(index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, subtitle: Option<&str>) -> RemotePickItem {
        RemotePickItem {
            id: id.to_owned(),
            label: id.to_owned(),
            subtitle: subtitle.map(str::to_owned),
            status: None,
            status_color: None,
            status_shape: None,
        }
    }

    fn items() -> Vec<RemotePickItem> {
        vec![
            item("api", Some("worker")),
            item("web", None),
            item("new tab", None),
            item("newsletter worker", None),
        ]
    }

    fn rows(indices: &[u32]) -> Vec<PickRow> {
        indices.iter().map(|&index| PickRow::Item { index }).collect()
    }

    #[test]
    fn a_blank_query_keeps_every_item_in_the_origins_order() {
        assert_eq!(pick_rows(items(), String::new(), false), rows(&[0, 1, 2, 3]));
        assert_eq!(pick_rows(items(), " \n\t".to_owned(), true), rows(&[0, 1, 2, 3]));
    }

    #[test]
    fn a_query_matches_labels_only_best_first() {
        assert_eq!(pick_rows(items(), "new".to_owned(), false), rows(&[2, 3]));
        assert_eq!(pick_rows(items(), "worker".to_owned(), false), rows(&[3]));
        let reversed = vec![item("newsletter worker", None), item("new", None)];
        assert_eq!(pick_rows(reversed, "new".to_owned(), false), rows(&[1, 0]));
    }

    #[test]
    fn an_unmatched_query_offers_its_trimmed_text_only_when_custom_answers_are_allowed() {
        assert!(pick_rows(items(), "zzz".to_owned(), false).is_empty());
        assert_eq!(
            pick_rows(items(), "  zzz top ".to_owned(), true),
            [PickRow::Custom {
                label: "Use \"zzz top\"".to_owned(),
                query: "zzz top".to_owned(),
            }]
        );
    }
}
