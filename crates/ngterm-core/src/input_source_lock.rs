//! When the keyboard layout is switched to the one Settings ▸ Key Mapping ▸ "Lock in terminals" names, and when the
//! earlier one comes back (upstream's `InputSourceLock.swift`). The frontend feeds it the system's state (whether a
//! terminal holds the keyboard, the layouts the system can select, the one selected) and performs the selection it
//! returns; the lock holds the layout that was selected when it engaged, so it can put it back.
//!
//! Layout ids are the platform's own: a Text Input Source id on macOS, the XKB layout name on Linux, a KLID on Windows.
//! The baseline lives in memory only: a crash leaves the locked layout selected, and a baseline persisted for the next
//! launch would overwrite a choice made elsewhere since.

use std::collections::HashSet;

/// The lock's state: the layout to return to while it holds an override.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputSourceLock {
    baseline: Option<String>,
}

impl InputSourceLock {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The layout to return to, `Some` exactly while the lock holds an override.
    #[must_use]
    pub fn baseline(&self) -> Option<&str> {
        self.baseline.as_deref()
    }

    /// The next layout to select, `None` when no change is needed. A `target` outside `enabled` releases an override
    /// held and cannot engage a new one.
    pub fn reconcile(
        &mut self,
        terminal_focused: bool,
        target: Option<&str>,
        enabled: &HashSet<String>,
        current: &str,
    ) -> Option<String> {
        let Some(target) = target.filter(|target| terminal_focused && enabled.contains(*target)) else {
            return self.release(current);
        };
        if self.baseline.is_none() {
            self.baseline = Some(current.to_owned());
        }
        (current != target).then(|| target.to_owned())
    }

    /// Ends the override; the layout to restore, `None` when none is held or it is already selected.
    pub fn release(&mut self, current: &str) -> Option<String> {
        let held = self.baseline.take()?;
        (held != current).then_some(held)
    }

    /// Drops an override whose selection the system refused, leaving nothing to restore.
    pub fn selection_failed(&mut self) {
        self.baseline = None;
    }
}

#[cfg(test)]
mod tests {
    //! Upstream's `InputSourceLockTests`, all nine.

    use super::*;

    const US: &str = "com.apple.keylayout.US";
    const RU: &str = "com.apple.keylayout.Russian-Phonetic";
    const DE: &str = "com.apple.keylayout.German";

    fn enabled(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    fn both() -> HashSet<String> {
        enabled(&[US, RU])
    }

    #[test]
    fn engaging_selects_the_target_and_remembers_the_earlier_source() {
        let mut lock = InputSourceLock::new();
        assert_eq!(lock.reconcile(true, Some(US), &both(), RU).as_deref(), Some(US));
        assert_eq!(lock.baseline(), Some(RU));
    }

    #[test]
    fn a_manual_switch_while_engaged_is_switched_back_and_the_baseline_kept() {
        let mut lock = InputSourceLock::new();
        lock.reconcile(true, Some(US), &both(), RU);
        assert_eq!(lock.reconcile(true, Some(US), &both(), US), None);
        assert_eq!(lock.reconcile(true, Some(US), &both(), RU).as_deref(), Some(US));
        assert_eq!(lock.baseline(), Some(RU));
    }

    #[test]
    fn losing_terminal_focus_restores_the_earlier_source_once() {
        let mut lock = InputSourceLock::new();
        lock.reconcile(true, Some(US), &both(), RU);
        assert_eq!(lock.reconcile(false, Some(US), &both(), US).as_deref(), Some(RU));
        assert_eq!(lock.baseline(), None);
        assert_eq!(lock.reconcile(false, Some(US), &both(), RU), None);
    }

    #[test]
    fn engaging_on_the_target_itself_selects_nothing_either_way() {
        let mut lock = InputSourceLock::new();
        assert_eq!(lock.reconcile(true, Some(US), &both(), US), None);
        assert_eq!(lock.baseline(), Some(US));
        assert_eq!(lock.reconcile(false, Some(US), &both(), US), None);
    }

    #[test]
    fn turning_the_setting_off_while_engaged_restores() {
        let mut lock = InputSourceLock::new();
        lock.reconcile(true, Some(US), &both(), RU);
        assert_eq!(lock.reconcile(true, None, &both(), US).as_deref(), Some(RU));
        assert_eq!(lock.baseline(), None);
    }

    #[test]
    fn changing_the_target_while_engaged_keeps_the_original_baseline() {
        let mut lock = InputSourceLock::new();
        let three = enabled(&[US, RU, DE]);
        lock.reconcile(true, Some(US), &three, RU);
        assert_eq!(lock.reconcile(true, Some(DE), &three, US).as_deref(), Some(DE));
        assert_eq!(lock.baseline(), Some(RU));
    }

    #[test]
    fn a_target_the_system_cannot_select_engages_nothing() {
        let mut lock = InputSourceLock::new();
        assert_eq!(lock.reconcile(true, Some(DE), &both(), RU), None);
        assert_eq!(lock.baseline(), None);
    }

    #[test]
    fn a_target_disabled_while_engaged_releases_the_override() {
        let mut lock = InputSourceLock::new();
        lock.reconcile(true, Some(US), &both(), RU);
        assert_eq!(lock.reconcile(true, Some(US), &enabled(&[RU]), US).as_deref(), Some(RU));
        assert_eq!(lock.baseline(), None);
    }

    #[test]
    fn a_refused_selection_leaves_nothing_to_restore() {
        let mut lock = InputSourceLock::new();
        lock.reconcile(true, Some(US), &both(), RU);
        lock.selection_failed();
        assert_eq!(lock.release(RU), None);
    }
}
