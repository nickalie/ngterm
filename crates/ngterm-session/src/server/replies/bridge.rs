//! A server-minted paste token names its source client, epoch and reply generation.
//! This map is never populated from program input or from permission request metadata.
use super::{Answer, ClientId, DELEGATION_WAIT, Instant, State, lock, vt};
use crate::protocol::{ClipboardPasteBridge, ClipboardStatus};

pub(super) struct PasteBridge {
    token: Vec<u8>,
    id: Option<String>,
    available: Vec<String>,
    client: ClientId,
    epoch: u64,
    generation: u32,
    deadline: Instant,
    consumed: bool,
}
impl std::fmt::Debug for PasteBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasteBridge")
            .field("id", &self.id)
            .field("consumed", &self.consumed)
            .finish_non_exhaustive()
    }
}

pub(super) enum Read {
    Ordinary,
    Answer(Answer),
    Fetch { client: ClientId, id: String },
}

impl State {
    pub(in crate::server) fn register_paste_bridge(
        &mut self,
        client: ClientId,
        epoch: u64,
        bridge: Option<&ClipboardPasteBridge>,
        bytes: &[u8],
    ) {
        let Some(body) = bytes.strip_prefix(b"\x1b]") else {
            return;
        };
        let Some(end) = body.windows(2).position(|pair| pair == b"\x1b\\") else {
            return;
        };
        let Some(packet) = crate::attach_tty::clipboard::Packet::parse(&body[..end]) else {
            return;
        };
        if packet.token.is_empty() {
            return;
        }
        if self.replies.paste_bridges.len() >= 32 {
            let evicted = self.replies.paste_bridges.remove(0);
            // Consuming another one-time grant can swap the Rust grant table's order.
            // Evicting this bridge must revoke its token even when that table still holds it.
            lock(&self.terminal.clipboard_grants()).consume(&evicted.token, vt::ClipboardDirection::Read);
        }
        self.replies.paste_bridges.push(PasteBridge {
            token: packet.token,
            id: bridge.map(|bridge| bridge.id.clone()),
            available: bridge.map_or_else(Vec::new, |bridge| bridge.available.iter().take(16).cloned().collect()),
            client,
            epoch,
            generation: self.replies.generation,
            deadline: Instant::now() + DELEGATION_WAIT,
            consumed: false,
        });
    }

    pub(super) fn bridge_read(&mut self, details: &vt::ClipboardReadDetails) -> Read {
        let Some(bridge) = self
            .replies
            .paste_bridges
            .iter_mut()
            .find(|bridge| bridge.token == details.pw)
        else {
            return Read::Ordinary;
        };
        let valid = !bridge.consumed
            && bridge.deadline > Instant::now()
            && self.replies.generation == bridge.generation
            && self.clients.epoch(bridge.client) == Some(bridge.epoch)
            && self.clients.overflowed(bridge.client) == Some(false)
            && lock(&self.terminal.clipboard_grants()).peek(&bridge.token, vt::ClipboardDirection::Read);
        if !valid {
            return Read::Answer(Answer::Denied);
        }
        let Some(id) = &bridge.id else {
            // Contents-only owner pastes read its clipboard, never another viewer's after an ownership change.
            if self.replies.owner != Some(bridge.client) {
                return Read::Answer(Answer::Denied);
            }
            bridge.consumed = !details.mimes.is_empty();
            return Read::Ordinary;
        };
        if details.mimes.is_empty() {
            return Read::Answer(Answer::Mime {
                contents: Vec::new(),
                available: bridge.available.clone(),
                remember: false,
                status: ClipboardStatus::Success,
            });
        }
        bridge.consumed = true;
        Read::Fetch {
            client: bridge.client,
            id: id.clone(),
        }
    }
}
