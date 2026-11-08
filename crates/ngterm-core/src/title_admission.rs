//! Which OSC titles a pane hands to the frontend (upstream's `TitleAdmission.swift`, #911). libghostty-vt delivers
//! every title a program writes, and a program writing one title in a loop would otherwise queue an event per write,
//! all applying the same string. A title identical to the last one queued is skipped while that one still waits; a
//! different title, or the same one after the queue drains, is queued.
//!
//! One value per pane, guarded by the caller: [`TitleAdmission::admit`] runs where the title is reported (the pane's
//! IO thread), [`TitleAdmission::complete`] where the queued title is taken (the frontend draining the pane's events).
//! The reserved-prefix titles a pane's attach client and remote wrapper write ([`is_notice`]) are reports, never the
//! pane's title, and bypass the admission.

use crate::live::lead::LeadNotice;
use crate::remote::reconnect::RemoteLinkNotice;

/// The titles of one pane queued and not yet taken.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitleAdmission {
    last_queued: Option<String>,
    pending: usize,
}

impl TitleAdmission {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `title` must be queued, recording it as the last queued title when it must.
    pub fn admit(&mut self, title: &str) -> bool {
        if self.pending > 0 && self.last_queued.as_deref() == Some(title) {
            return false;
        }
        title.clone_into(self.last_queued.get_or_insert_with(String::new));
        self.pending += 1;
        true
    }

    /// One queued title was taken off the queue. A completion with nothing queued changes nothing.
    pub fn complete(&mut self) {
        self.pending = self.pending.saturating_sub(1);
    }
}

/// Whether `title` is a lead or remote-link notice (a report under a reserved prefix), which bypasses the admission.
#[must_use]
pub fn is_notice(title: &str) -> bool {
    LeadNotice::parse(title).is_some() || RemoteLinkNotice::parse(title).is_some()
}

/// Ported from upstream's `TitleAdmissionTests`.
#[cfg(test)]
mod tests {
    use super::*;

    fn admitted(admission: &mut TitleAdmission, titles: &[&str]) -> Vec<bool> {
        titles.iter().map(|title| admission.admit(title)).collect()
    }

    #[test]
    fn a_duplicate_is_skipped_while_the_first_is_pending() {
        let mut admission = TitleAdmission::new();
        assert_eq!(
            admitted(&mut admission, &["build", "build", "build"]),
            [true, false, false]
        );
    }

    #[test]
    fn a_changed_title_between_two_equal_ones_queues_all_three() {
        let mut admission = TitleAdmission::new();
        assert_eq!(admitted(&mut admission, &["T", "U", "T"]), [true, true, true]);
    }

    #[test]
    fn the_same_title_queues_again_once_the_queue_drains() {
        let mut admission = TitleAdmission::new();
        let queued = admitted(&mut admission, &["T", "U"]);
        admission.complete();
        let while_pending = admission.admit("U");
        admission.complete();
        let after_drain = admission.admit("U");
        assert_eq!(queued, [true, true]);
        assert!(!while_pending);
        assert!(after_drain);
    }

    #[test]
    fn each_pane_decides_on_its_own_titles() {
        let (mut left, mut right) = (TitleAdmission::new(), TitleAdmission::new());
        let first = [left.admit("T"), right.admit("T"), left.admit("T")];
        right.complete();
        let right_again = right.admit("T");
        assert_eq!(first, [true, true, false]);
        assert!(right_again);
    }

    #[test]
    fn an_extra_complete_never_readmits_a_pending_duplicate() {
        let mut admission = TitleAdmission::new();
        admission.complete();
        assert_eq!(admitted(&mut admission, &["T", "T"]), [true, false]);
    }

    #[test]
    fn lead_and_remote_link_notices_are_told_from_titles() {
        assert!(is_notice("ngterm-live-role;n1:leader:1"));
        assert!(is_notice(&RemoteLinkNotice::title("n1")));
        for title in ["build", "", "ngterm-live-role;", "ngterm-remote;n1"] {
            assert!(!is_notice(title), "{title}");
        }
    }
}
