//! How each HTML page ended (upstream's `HtmlPageOutcomes.swift`), keyed by page id rather than by slot, so a caller
//! blocked on a page reads the answer after the page, its session or its window are gone. Open pages bound the pending
//! set; finished outcomes keep the most recent [`RETAINED_LIMIT`].

use std::collections::{HashSet, VecDeque};

use ngterm_proto::{ControlHtmlPageOutcome, ControlHtmlPageOutcomeState};
use uuid::Uuid;

use crate::pane_env::uuid_string;

/// How many finished outcomes stay readable (upstream's `retainedLimit`).
pub const RETAINED_LIMIT: usize = 32;

/// The outcomes of the pages this app run opened.
#[derive(Debug, Default)]
pub struct HtmlPageOutcomes {
    pending: HashSet<Uuid>,
    finished: VecDeque<(Uuid, ControlHtmlPageOutcome)>,
}

impl HtmlPageOutcomes {
    /// Starts tracking an opened page as pending.
    pub fn register(&mut self, id: Uuid) {
        self.pending.insert(id);
    }

    /// Answers a pending page with `value`; false when it is not waiting, so the first answer stands.
    pub fn submit(&mut self, id: Uuid, value: &str) -> bool {
        self.finish(
            id,
            ControlHtmlPageOutcome {
                page_id: uuid_string(id),
                outcome: ControlHtmlPageOutcomeState::Submitted,
                value: Some(value.to_owned()),
            },
        )
    }

    /// Records a pending page closed without an answer; a no-op for one already answered.
    pub fn dismiss(&mut self, id: Uuid) {
        self.finish(
            id,
            ControlHtmlPageOutcome {
                page_id: uuid_string(id),
                outcome: ControlHtmlPageOutcomeState::Dismissed,
                value: None,
            },
        );
    }

    /// Answers pending page `id` the way another machine answered the page it shows (upstream's `settle`): submitted
    /// with that value, otherwise dismissed.
    pub fn settle(&mut self, id: Uuid, like: Option<&ControlHtmlPageOutcome>) {
        match like {
            Some(ControlHtmlPageOutcome {
                outcome: ControlHtmlPageOutcomeState::Submitted,
                value: Some(value),
                ..
            }) => {
                self.submit(id, value);
            }
            _ => self.dismiss(id),
        }
    }

    /// Whether page `id` is still open and unanswered.
    #[must_use]
    pub fn is_pending(&self, id: Uuid) -> bool {
        self.pending.contains(&id)
    }

    /// Where page `id` stands; `None` for a page never opened or whose outcome aged out.
    #[must_use]
    pub fn outcome(&self, id: Uuid) -> Option<ControlHtmlPageOutcome> {
        if self.pending.contains(&id) {
            return Some(ControlHtmlPageOutcome {
                page_id: uuid_string(id),
                outcome: ControlHtmlPageOutcomeState::Pending,
                value: None,
            });
        }
        self.finished
            .iter()
            .rev()
            .find(|(finished, _)| *finished == id)
            .map(|(_, outcome)| outcome.clone())
    }

    fn finish(&mut self, id: Uuid, outcome: ControlHtmlPageOutcome) -> bool {
        if !self.pending.remove(&id) {
            return false;
        }
        self.finished.push_back((id, outcome));
        while self.finished.len() > RETAINED_LIMIT {
            self.finished.pop_front();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(outcomes: &HtmlPageOutcomes, id: Uuid) -> Option<ControlHtmlPageOutcomeState> {
        outcomes.outcome(id).map(|outcome| outcome.outcome)
    }

    fn value(outcomes: &HtmlPageOutcomes, id: Uuid) -> Option<String> {
        outcomes.outcome(id).and_then(|outcome| outcome.value)
    }

    /// Upstream's `settleCopiesAnotherMacsSubmissionAndDismissesOtherwise`.
    #[test]
    fn settle_copies_another_machines_submission_and_dismisses_otherwise() {
        let mut outcomes = HtmlPageOutcomes::default();
        let (answered, unknown) = (Uuid::new_v4(), Uuid::new_v4());
        outcomes.register(answered);
        outcomes.register(unknown);
        let origin = ControlHtmlPageOutcome {
            page_id: "origin".to_owned(),
            outcome: ControlHtmlPageOutcomeState::Submitted,
            value: Some("yes".to_owned()),
        };
        outcomes.settle(answered, Some(&origin));
        assert_eq!(
            outcomes.outcome(answered),
            Some(ControlHtmlPageOutcome {
                page_id: uuid_string(answered),
                outcome: ControlHtmlPageOutcomeState::Submitted,
                value: Some("yes".to_owned()),
            })
        );
        outcomes.settle(unknown, None);
        assert_eq!(state(&outcomes, unknown), Some(ControlHtmlPageOutcomeState::Dismissed));
    }

    #[test]
    fn a_registered_page_is_pending_until_it_ends() {
        let mut outcomes = HtmlPageOutcomes::default();
        let id = Uuid::new_v4();
        assert_eq!(outcomes.outcome(id), None);
        assert!(!outcomes.is_pending(id));
        outcomes.register(id);
        assert!(outcomes.is_pending(id));
        assert_eq!(
            outcomes.outcome(id),
            Some(ControlHtmlPageOutcome {
                page_id: uuid_string(id),
                outcome: ControlHtmlPageOutcomeState::Pending,
                value: None,
            })
        );
        assert!(outcomes.submit(id, "feature-x"));
        assert!(!outcomes.is_pending(id));
        assert_eq!(
            outcomes.outcome(id),
            Some(ControlHtmlPageOutcome {
                page_id: uuid_string(id),
                outcome: ControlHtmlPageOutcomeState::Submitted,
                value: Some("feature-x".to_owned()),
            })
        );
    }

    /// Upstream's `HtmlPageOutcomesTests.eachTransitionHappensOnce`.
    #[test]
    fn each_transition_happens_once() {
        let mut outcomes = HtmlPageOutcomes::default();
        let id = Uuid::new_v4();
        outcomes.register(id);
        assert!(outcomes.submit(id, "first"));
        assert!(!outcomes.submit(id, "second"));
        outcomes.dismiss(id);
        assert_eq!(state(&outcomes, id), Some(ControlHtmlPageOutcomeState::Submitted));
        assert_eq!(value(&outcomes, id).as_deref(), Some("first"));
        let unknown = Uuid::new_v4();
        assert!(!outcomes.submit(unknown, "x"));
        outcomes.dismiss(unknown);
        assert_eq!(outcomes.outcome(unknown), None);
    }

    /// Upstream's `HtmlPageOutcomesTests.anEmptyOrMultilineValueIsARealAnswer`, at the record.
    #[test]
    fn an_empty_or_multiline_value_is_a_real_answer() {
        let mut outcomes = HtmlPageOutcomes::default();
        for answer in ["", "a\nb"] {
            let id = Uuid::new_v4();
            outcomes.register(id);
            assert!(outcomes.submit(id, answer));
            assert_eq!(value(&outcomes, id).as_deref(), Some(answer));
        }
    }

    /// Upstream's `HtmlPageOutcomesTests.aReadNeverConsumesTheOutcome`.
    #[test]
    fn a_read_never_consumes_the_outcome() {
        let mut outcomes = HtmlPageOutcomes::default();
        let id = Uuid::new_v4();
        outcomes.register(id);
        outcomes.dismiss(id);
        let dismissed = Some(ControlHtmlPageOutcome {
            page_id: uuid_string(id),
            outcome: ControlHtmlPageOutcomeState::Dismissed,
            value: None,
        });
        assert_eq!(outcomes.outcome(id), dismissed);
        assert_eq!(outcomes.outcome(id), dismissed);
    }

    /// Upstream's `HtmlPageOutcomesTests.retentionEvictsOldFinishedOutcomesButNeverAPendingOne`.
    #[test]
    fn retention_evicts_old_finished_outcomes_but_never_a_pending_one() {
        let mut outcomes = HtmlPageOutcomes::default();
        let waiting = Uuid::new_v4();
        outcomes.register(waiting);
        let finished: Vec<Uuid> = (0..=RETAINED_LIMIT).map(|_| Uuid::new_v4()).collect();
        for id in &finished {
            outcomes.register(*id);
            outcomes.dismiss(*id);
        }
        assert_eq!(RETAINED_LIMIT, 32);
        assert_eq!(outcomes.outcome(finished[0]), None);
        assert_eq!(
            state(&outcomes, finished[1]),
            Some(ControlHtmlPageOutcomeState::Dismissed)
        );
        assert_eq!(state(&outcomes, waiting), Some(ControlHtmlPageOutcomeState::Pending));
    }
}
