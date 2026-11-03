//! Pane addresses and the app-run mailbox bus (upstream's `MessageAddress.swift` and `MessageBus.swift`).
//! The window library shares one bus with its stores, as it shares Open Recent; a standalone store has its own.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use ngterm_proto::{BusMessage, ControlResult, MessageRejection, MessageRejectionReason};
use uuid::Uuid;

use crate::error::ControlError;
use crate::model::StatusPane;
use crate::pane_env::uuid_string;
use crate::persist::parse_uuid_string;

pub(crate) const MAX_BODY_BYTES: usize = 64 * 1024;
pub(crate) const MAX_MESSAGES: usize = 256;
pub(crate) const MAX_MAILBOX_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_BATCH: usize = 32;

/// A recipient may name a role; an identity must pin the surface token, since swaps repoint roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MessagePane {
    Id(Uuid),
    Role(StatusPane),
}

impl MessagePane {
    pub(crate) fn text(self) -> String {
        match self {
            Self::Id(id) => uuid_string(id),
            Self::Role(role) => role.as_str().to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MessageAddress<'a> {
    pub(crate) session: &'a str,
    pub(crate) pane: MessagePane,
}

impl<'a> MessageAddress<'a> {
    pub(crate) fn parse(text: &'a str, identity: bool) -> Result<Self, ControlError> {
        let parts: Vec<&str> = text.split(':').collect();
        if parts.len() == 1 && !text.is_empty() {
            return Err(ControlError::rejected(format!(
                "address \"{text}\" names a session, not a pane"
            )));
        }
        let malformed = || ControlError::rejected(format!("invalid address \"{text}\": expected SESSION:PANE"));
        let [session, pane] = parts.as_slice() else {
            return Err(malformed());
        };
        if session.is_empty() || pane.is_empty() {
            return Err(malformed());
        }
        let pane = if let Some(id) = parse_uuid_string(pane) {
            MessagePane::Id(id)
        } else {
            let role = StatusPane::from_control_name(pane).ok_or_else(malformed)?;
            if identity {
                return Err(ControlError::rejected(format!(
                    "identity \"{text}\" must name a pane id, not a role"
                )));
            }
            MessagePane::Role(role)
        };
        Ok(Self { session, pane })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MessageRecipient {
    pub(crate) pane: Uuid,
    pub(crate) address: String,
}

impl MessageRecipient {
    pub(crate) fn new(session: Uuid, pane: Uuid) -> Self {
        Self {
            pane,
            address: format!("{}:{}", uuid_string(session), uuid_string(pane)),
        }
    }
}

/// One main-thread bus shared by the library's open stores. It never persists or enters snapshots.
#[derive(Debug, Clone, Default)]
pub(crate) struct MessageBus(Rc<RefCell<HashMap<Uuid, VecDeque<BusMessage>>>>);

impl MessageBus {
    pub(crate) fn send(
        &self,
        body: &str,
        from: Option<&str>,
        recipients: &[MessageRecipient],
        now: f64,
    ) -> Result<ControlResult, ControlError> {
        validate_body(body)?;
        let id = uuid_string(Uuid::new_v4());
        let mut delivered = Vec::new();
        let mut rejected = Vec::new();
        let mut boxes = self.0.borrow_mut();
        for recipient in recipients {
            let mailbox = boxes.entry(recipient.pane).or_default();
            let bytes: usize = mailbox.iter().map(|message| message.body.len()).sum();
            if mailbox.len() >= MAX_MESSAGES || bytes + body.len() > MAX_MAILBOX_BYTES {
                rejected.push(MessageRejection {
                    address: recipient.address.clone(),
                    reason: MessageRejectionReason::MailboxFull,
                });
                continue;
            }
            mailbox.push_back(BusMessage {
                id: id.clone(),
                from: from.map(str::to_owned),
                to: recipient.address.clone(),
                ts: now,
                body: body.to_owned(),
            });
            delivered.push(recipient.address.clone());
        }
        Ok(ControlResult {
            message_id: Some(id),
            delivered: Some(delivered),
            rejected: Some(rejected),
            ..ControlResult::default()
        })
    }

    pub(crate) fn read(&self, pane: Uuid, limit: usize, ack: bool) -> ControlResult {
        let mut boxes = self.0.borrow_mut();
        let mailbox = boxes.entry(pane).or_default();
        let messages: Vec<BusMessage> = mailbox.iter().take(limit.min(MAX_BATCH)).cloned().collect();
        let remaining = mailbox.len() - messages.len();
        let acked = ack.then(|| messages.iter().map(|message| message.id.clone()).collect());
        if ack {
            mailbox.drain(..messages.len());
        }
        ControlResult {
            messages: Some(messages),
            remaining: Some(remaining),
            acked,
            ..ControlResult::default()
        }
    }

    pub(crate) fn ack(&self, pane: Uuid, ids: &[Uuid]) -> Vec<String> {
        let wanted: HashSet<Uuid> = ids.iter().copied().collect();
        let mut boxes = self.0.borrow_mut();
        let Some(mailbox) = boxes.get_mut(&pane) else {
            return Vec::new();
        };
        let mut removed = Vec::new();
        mailbox.retain(|message| {
            let remove = Uuid::parse_str(&message.id).is_ok_and(|id| wanted.contains(&id));
            if remove {
                removed.push(message.id.clone());
            }
            !remove
        });
        removed
    }

    pub(crate) fn pending(&self, pane: Uuid) -> usize {
        self.0.borrow().get(&pane).map_or(0, VecDeque::len)
    }

    pub(crate) fn dispose(&self, pane: Uuid) {
        self.0.borrow_mut().remove(&pane);
    }
}

pub(crate) fn validate_body(body: &str) -> Result<(), ControlError> {
    if body.len() > MAX_BODY_BYTES {
        Err(ControlError::rejected(format!(
            "message body too large ({} bytes, max {MAX_BODY_BYTES})",
            body.len()
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
