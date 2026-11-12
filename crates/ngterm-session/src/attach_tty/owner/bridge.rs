//! Authoritative server consent over a negotiated, private attach channel.
use super::*;
use ngterm_core::attach_bridge::{Envelope, Message, VERSION};

#[derive(Debug)]
pub(super) struct Pending {
    id: u64,
    request: Delegated,
    pub(super) deadline: Instant,
}

impl Relay {
    pub fn enable_bridge(&mut self, nonce: Option<String>) {
        self.bridge_nonce = nonce;
    }
    pub(super) fn bridge_hello(&mut self, fx: &mut Effects) {
        let Some(nonce) = &self.bridge_nonce else {
            return;
        };
        if self.bridge_hello_sent {
            return;
        }
        self.bridge_hello_sent = true;
        fx.stdout.extend(
            Envelope {
                version: VERSION,
                nonce: nonce.clone(),
                message: Message::Hello,
            }
            .encode(),
        );
    }
    pub(super) fn bridge_request(&mut self, nonce: String, request: Delegated, now: Instant, fx: &mut Effects) {
        if self.bridge_pending.is_some() {
            fx.outgoing.push(Outgoing::Answer(request, DelegatedReply::Denied));
            return;
        }
        let id = self.bridge_next_id;
        let Some(next_id) = id.checked_add(1) else {
            fx.outgoing.push(Outgoing::Answer(request, DelegatedReply::Denied));
            return;
        };
        self.bridge_next_id = next_id;
        let bytes = Envelope {
            version: VERSION,
            nonce,
            message: Message::Request {
                id,
                epoch: request.epoch,
                request: request.request.clone(),
            },
        }
        .encode();
        self.bridge_pending = Some(Pending {
            id,
            request,
            deadline: now + DELEGATION_WAIT,
        });
        self.inject(
            Injection {
                bytes,
                fence: None,
                clipboard_read: true,
                completion: None,
            },
            now,
            fx,
        );
    }
    pub(super) fn bridge_answer(&mut self, envelope: Envelope, fx: &mut Effects) {
        if self.bridge_nonce.as_ref() != Some(&envelope.nonce) {
            return;
        }
        match envelope.message {
            Message::Reply { id, epoch, reply } => {
                if self
                    .bridge_pending
                    .as_ref()
                    .is_none_or(|pending| pending.id != id || pending.request.epoch != epoch)
                {
                    return;
                }
                if let Some(pending) = self.bridge_pending.take() {
                    fx.outgoing.push(Outgoing::Answer(pending.request, reply));
                }
            }
            Message::Paste(paste) => {
                fx.outgoing.push(Outgoing::Paste(paste));
            }
            _ => {}
        }
    }
    pub(super) fn bridge_expire(&mut self, now: Instant, fx: &mut Effects) {
        if self
            .bridge_pending
            .as_ref()
            .is_some_and(|pending| pending.deadline <= now)
        {
            if let Some(pending) = self.bridge_pending.take() {
                fx.outgoing
                    .push(Outgoing::Answer(pending.request, DelegatedReply::Denied));
            }
            self.pending.retain(|injection| !injection.clipboard_read);
            self.unqueued();
        }
    }
}

#[cfg(test)]
mod tests;
