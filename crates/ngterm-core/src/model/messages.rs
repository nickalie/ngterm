//! Live pane addressing, message events and mailbox disposal (`AppStore+Messages.swift`).

use ngterm_proto::{EventKind, EventPayload};
use uuid::Uuid;

use super::{AppStore, Session, StatusPane};
use crate::messages::{MessagePane, MessageRecipient};
use crate::pane_env::uuid_string;

impl AppStore {
    pub(crate) fn message_recipient(&self, session: Uuid, pane: MessagePane) -> Option<MessageRecipient> {
        let session = self.session(session)?;
        live_panes(session).into_iter().find_map(|(role, id)| {
            let found = match pane {
                MessagePane::Id(pane) => pane == id,
                MessagePane::Role(pane) => pane == role,
            };
            found.then(|| MessageRecipient::new(session.id(), id))
        })
    }

    pub(crate) fn message_recipients(&self, workspace: Option<Uuid>) -> Vec<MessageRecipient> {
        self.workspaces()
            .iter()
            .filter(|item| workspace.is_none_or(|id| id == item.id()))
            .flat_map(|workspace| {
                workspace.sessions.iter().flat_map(|session| {
                    live_panes(session)
                        .into_iter()
                        .map(|(_, pane)| MessageRecipient::new(session.id(), pane))
                })
            })
            .collect()
    }

    pub(crate) fn emit_message_queued(&mut self, id: &str, from: Option<&str>, pane: Uuid) {
        let found = self
            .sessions()
            .find(|session| live_panes(session).iter().any(|(_, id)| *id == pane));
        let Some(session) = found else { return };
        let session_id = session.id();
        let workspace = self.workspace_for_session(session_id).map(super::Workspace::id);
        let payload = EventPayload {
            name: Some(session.display_name()),
            pane_id: Some(uuid_string(pane)),
            from: from.map(str::to_owned),
            message_id: Some(id.to_owned()),
            ..EventPayload::default()
        };
        self.emit(EventKind::Message, workspace, Some(session_id), payload);
    }

    /// Leaving a visible tree drops mail even when a soft-close undo keeps the surfaces alive.
    pub(crate) fn dispose_session_mailboxes(&self, id: Uuid) {
        for role in [StatusPane::Left, StatusPane::Right, StatusPane::Scratch] {
            self.dispose_pane_mailbox(id, role);
        }
    }

    pub(crate) fn dispose_pane_mailbox(&self, id: Uuid, role: StatusPane) {
        if let Some(session) = self.session(id) {
            if let Some(surface) = session.pane_slot(role).get()
                && let Ok(pane) = Uuid::parse_str(surface.pane_token())
            {
                self.messages.dispose(pane);
            }
            let pane = match role {
                StatusPane::Left => Some(session.pane_identity),
                StatusPane::Right => session.split_pane_identity,
                StatusPane::Scratch => None,
            };
            if let Some(pane) = pane {
                self.messages.dispose(pane);
            }
        }
    }
}

fn live_panes(session: &Session) -> Vec<(StatusPane, Uuid)> {
    if session.remote_host().is_some() {
        return Vec::new();
    }
    [StatusPane::Left, StatusPane::Right, StatusPane::Scratch]
        .into_iter()
        .filter_map(|role| {
            let surface = session.pane_slot(role).get()?;
            if !surface.is_realized() {
                return None;
            }
            Some((role, Uuid::parse_str(surface.pane_token()).ok()?))
        })
        .collect()
}
