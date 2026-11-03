//! Sidebar info mutation, following upstream AppStore+SessionState.swift.
use super::AppStore;
use crate::session_info::SessionInfo;
use uuid::Uuid;

impl AppStore {
    /// Changes runtime local info only. Identical values do not emit tree.changed; the command still rearms expiry.
    pub fn set_info(&mut self, info: Option<SessionInfo>, id: Uuid) -> bool {
        let Some(session) = self.session_mut(id) else {
            return false;
        };
        if session.info == info {
            return false;
        }
        let previous = session.effective_info().cloned();
        if info.is_none() {
            session.discard_info();
        } else {
            session.info = info;
        }
        if previous.as_ref() != session.effective_info() {
            self.schedule_tree_changed();
        }
        true
    }

    pub(crate) fn discard_timed_info(&mut self, id: Uuid) {
        if self
            .session(id)
            .and_then(|session| session.info.as_ref())
            .is_some_and(|info| info.hide_after > 0.0)
        {
            self.set_info(None, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NewSession, SaveRequest};

    #[test]
    fn an_equal_local_override_or_clear_does_not_change_the_effective_value() {
        let mut store = AppStore::new();
        let workspace = store.add_workspace("work");
        let id = store.add_session(workspace, NewSession::at_cwd("/tmp")).unwrap();
        let info = SessionInfo::make("same", false, 0.0).unwrap();
        store.session_mut(id).unwrap().mirrored_info = Some(info.clone());
        store.take_outbox();
        assert!(store.set_info(Some(info), id));
        assert!(store.take_outbox().is_empty());
        assert!(store.set_info(None, id));
        let outbox = store.take_outbox();
        assert!(outbox.events.is_empty());
        assert_eq!(outbox.save, SaveRequest::None);
        assert_eq!(store.session(id).unwrap().effective_info().unwrap().source, "same");
    }
}
