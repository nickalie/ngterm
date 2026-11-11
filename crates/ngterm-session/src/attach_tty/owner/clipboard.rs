//! Correlated native clipboard operations; a late id never completes another operation.
use super::{
    DELEGATION_WAIT, Delegated, DelegatedReply, DelegatedRequest, Effects, Injection, Instant, Outgoing, Relay,
};
use crate::attach_tty::clipboard::{Packet, read_query, write_query};
use crate::protocol::{ClipboardRepresentation, ClipboardStatus};

pub(super) const PROBE_ID: &str = "ngterm-clipboard-probe";

#[derive(Debug)]
pub(super) struct Operation {
    id: String,
    request: Delegated,
    deadline: Instant,
    started: bool,
    contents: Vec<ClipboardRepresentation>,
    available: Vec<String>,
}

#[derive(Debug, Default)]
pub(super) struct Native {
    pub supported: bool,
    pub pending: Option<Operation>,
    paste: Option<Paste>,
    bridges: Vec<Paste>,
}

struct Paste {
    id: String,
    token: Vec<u8>,
    available: Vec<String>,
    primary: bool,
    deadline: Instant,
}
impl std::fmt::Debug for Paste {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Paste").field("id", &self.id).finish_non_exhaustive()
    }
}

impl Native {
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|pending| pending.deadline)
    }
    pub fn invalidate(&mut self) {
        self.pending = None;
        self.paste = None;
        self.bridges.clear();
    }
}

impl Relay {
    pub fn reply_owner(&mut self, owner: bool) {
        self.reply_owner = owner;
        if !owner {
            self.native_clipboard.invalidate();
            self.bridge_pending = None;
            self.pending.retain(|injection| !injection.clipboard_read);
            self.unqueued();
        }
    }
    pub(super) fn native_delegated(&mut self, request: Delegated, now: Instant, fx: &mut Effects) {
        if self.native_clipboard.pending.is_some() || self.outer_read.is_some() {
            fx.outgoing.push(Outgoing::Answer(request, DelegatedReply::Denied));
            return;
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let alias = request
            .native_alias
            .as_ref()
            .map(|alias| alias.0.as_bytes())
            .unwrap_or_default();
        let bytes = match &request.request {
            DelegatedRequest::ClipboardRead { selection, list_only } => read_query(
                &id,
                selection,
                &if *list_only {
                    Vec::new()
                } else {
                    vec!["text/plain".into()]
                },
                *list_only,
                &[],
            ),
            DelegatedRequest::ClipboardReadMime {
                selection,
                mimes,
                list,
                bridge,
                ..
            } => {
                let token = if let Some(bridge) = bridge {
                    let Some(index) = self
                        .native_clipboard
                        .bridges
                        .iter()
                        .position(|paste| paste.id == *bridge && paste.deadline > now)
                    else {
                        fx.outgoing.push(Outgoing::Answer(request, DelegatedReply::Denied));
                        return;
                    };
                    self.native_clipboard.bridges.remove(index).token
                } else {
                    Vec::new()
                };
                read_query(
                    &id,
                    selection,
                    mimes,
                    *list,
                    if token.is_empty() { alias } else { &token },
                )
            }
            DelegatedRequest::ClipboardWrite { .. } => write_query(&id, &request.request, alias),
        };
        self.native_clipboard.pending = Some(Operation {
            id,
            request,
            deadline: now + DELEGATION_WAIT,
            started: false,
            contents: Vec::new(),
            available: Vec::new(),
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

    pub(super) fn native_answer(&mut self, packet: Packet, now: Instant, fx: &mut Effects) {
        if packet.id == PROBE_ID && packet.operation == "read" {
            self.native_clipboard.supported |= packet.status == "OK";
            return;
        }
        if packet.id.is_empty() && !packet.token.is_empty() {
            self.native_paste(packet, now, fx);
            return;
        }
        let Some(pending) = &mut self.native_clipboard.pending else {
            return;
        };
        if packet.id != pending.id {
            return;
        }
        let write = matches!(pending.request.request, DelegatedRequest::ClipboardWrite { .. });
        if packet.operation != if write { "write" } else { "read" } {
            return;
        }
        match packet.status.as_str() {
            "OK" if !write && !pending.started => pending.started = true,
            "DATA" if !write && pending.started => {
                if !pending.add(packet) {
                    self.native_finish(ClipboardStatus::InvalidData, fx);
                }
            }
            _ => {
                if let Some(status) = packet.outcome() {
                    let status = if !write && status == ClipboardStatus::Success && !pending.started {
                        ClipboardStatus::InvalidData
                    } else {
                        status
                    };
                    self.native_finish(status, fx);
                }
            }
        }
    }

    fn native_finish(&mut self, status: ClipboardStatus, fx: &mut Effects) {
        let Some(pending) = self.native_clipboard.pending.take() else {
            return;
        };
        self.pending.retain(|injection| !injection.clipboard_read);
        self.unqueued();
        let reply = match pending.request.request {
            DelegatedRequest::ClipboardWrite { .. } => DelegatedReply::ClipboardWrite {
                status,
                remember: false,
            },
            DelegatedRequest::ClipboardRead { .. } if status == ClipboardStatus::Success => DelegatedReply::Clipboard {
                text: pending
                    .contents
                    .iter()
                    .find(|part| part.mime == "text/plain")
                    .filter(|part| !part.data.is_empty())
                    .map(|part| String::from_utf8_lossy(&part.data).into_owned()),
            },
            _ => DelegatedReply::ClipboardMime {
                contents: if status == ClipboardStatus::Success {
                    pending.contents
                } else {
                    Vec::new()
                },
                available: if status == ClipboardStatus::Success {
                    pending.available
                } else {
                    Vec::new()
                },
                remember: false,
                status,
            },
        };
        fx.outgoing.push(Outgoing::Answer(pending.request, reply));
    }

    pub(super) fn native_expire(&mut self, now: Instant, fx: &mut Effects) {
        if self.native_clipboard.deadline().is_some_and(|deadline| now >= deadline) {
            self.native_finish(ClipboardStatus::Denied, fx);
        }
    }

    fn native_paste(&mut self, packet: Packet, now: Instant, fx: &mut Effects) {
        if packet.operation != "read" {
            return;
        }
        if packet.status == "OK" {
            self.native_clipboard.paste = Some(Paste {
                id: uuid::Uuid::new_v4().simple().to_string(),
                token: packet.token,
                available: Vec::new(),
                primary: packet.primary,
                deadline: now + DELEGATION_WAIT,
            });
            return;
        }
        let Some(paste) = &mut self.native_clipboard.paste else {
            return;
        };
        if paste.token != packet.token || paste.deadline <= now {
            self.native_clipboard.paste = None;
            return;
        }
        if packet.status == "DATA" && packet.mime == "." {
            let Ok(types) = std::str::from_utf8(&packet.data) else {
                self.native_clipboard.paste = None;
                return;
            };
            paste.available = types.split_whitespace().take(16).map(str::to_owned).collect();
        } else if packet.status == "DONE" {
            let Some(paste) = self.native_clipboard.paste.take() else {
                return;
            };
            fx.outgoing.push(Outgoing::Paste(crate::protocol::ClipboardPaste {
                bridge: Some(crate::protocol::ClipboardPasteBridge {
                    id: paste.id.clone(),
                    available: paste.available.clone(),
                }),
                contents: Vec::new(),
                text_source: false,
                selection: if paste.primary { "p" } else { "c" }.into(),
                allow_unsafe: false,
            }));
            if self.native_clipboard.bridges.len() >= 32 {
                self.native_clipboard.bridges.remove(0);
            }
            self.native_clipboard.bridges.push(paste);
        }
    }
}

impl Operation {
    fn add(&mut self, packet: Packet) -> bool {
        if packet.mime == "." {
            let Ok(list) = String::from_utf8(packet.data) else {
                return false;
            };
            self.available = list.split_whitespace().take(16).map(str::to_owned).collect();
            return true;
        }
        let requested = match &self.request.request {
            DelegatedRequest::ClipboardReadMime { mimes, .. } => mimes.iter().take(4).any(|mime| *mime == packet.mime),
            DelegatedRequest::ClipboardRead { .. } => packet.mime == "text/plain",
            DelegatedRequest::ClipboardWrite { .. } => false,
        };
        if !requested {
            return false;
        }
        let size: usize = self.contents.iter().map(|part| part.data.len()).sum();
        if size.saturating_add(packet.data.len()) > ngterm_core::attach_bridge::DATA_LIMIT {
            return false;
        }
        if let Some(part) = self.contents.iter_mut().find(|part| part.mime == packet.mime) {
            part.data.extend(packet.data);
        } else {
            self.contents.push(ClipboardRepresentation {
                mime: packet.mime,
                data: packet.data,
            });
        }
        true
    }
}

#[cfg(test)]
mod tests;
