//! The private attach bridge, enabled only on designated viewer panes.
use crate::pane::{OpenRequest, Pane, Worker};
use crate::{ClipboardAnswer, ClipboardLocation, ClipboardReadRequest, PaneEvent, ReadScope};
use ngterm_core::attach_bridge::{Envelope, Message, VERSION};
use ngterm_core::clipboard_wire as wire;
use ngterm_core::live::lead::LeadNotice;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use wire::{DelegatedReply, DelegatedRequest};

pub(crate) fn delegated_event(id: u64, request: &DelegatedRequest) -> PaneEvent {
    let location = |selection: &str| match selection {
        "s" => ClipboardLocation::Selection,
        "p" => ClipboardLocation::Primary,
        _ => ClipboardLocation::Standard,
    };
    match request {
        DelegatedRequest::ClipboardRead { selection, list_only } => PaneEvent::ClipboardRead(ClipboardReadRequest {
            id,
            location: location(selection),
            scope: if *list_only {
                ReadScope::ListOnly
            } else {
                ReadScope::Data
            },
            mimes: if *list_only {
                Vec::new()
            } else {
                vec!["text/plain".into()]
            },
            list: *list_only,
            name: String::new(),
            granted: false,
            can_remember: false,
            permission_scope: crate::ClipboardPermissionScope::Osc52,
        }),
        DelegatedRequest::ClipboardReadMime {
            bridge: _,
            selection,
            mimes,
            list,
            name,
            granted,
            can_remember,
        } => PaneEvent::ClipboardRead(ClipboardReadRequest {
            id,
            location: location(selection),
            scope: if mimes.is_empty() {
                ReadScope::ListOnly
            } else {
                ReadScope::Data
            },
            mimes: mimes.clone(),
            list: *list,
            name: name.clone(),
            granted: *granted,
            can_remember: *can_remember,
            permission_scope: crate::ClipboardPermissionScope::Kitty,
        }),
        DelegatedRequest::ClipboardWrite {
            selection,
            contents,
            name,
            granted,
            can_remember,
            kitty,
        } => PaneEvent::ClipboardWrite(crate::ClipboardWrite {
            id,
            location: location(selection),
            contents: contents
                .iter()
                .map(|content| crate::ClipboardContent {
                    mime: content.mime.clone(),
                    data: content.data.clone(),
                })
                .collect(),
            name: name.clone(),
            granted: *granted,
            can_remember: *can_remember,
            permission_scope: if *kitty {
                crate::ClipboardPermissionScope::Kitty
            } else {
                crate::ClipboardPermissionScope::Osc52
            },
        }),
    }
}

pub(crate) fn answer_reply(answer: ClipboardAnswer) -> DelegatedReply {
    match answer {
        ClipboardAnswer::Text(text) => DelegatedReply::Clipboard { text },
        ClipboardAnswer::Denied => DelegatedReply::Denied,
        ClipboardAnswer::Data {
            contents,
            available,
            remember,
        } => DelegatedReply::ClipboardMime {
            contents: contents
                .into_iter()
                .map(|content| wire::ClipboardRepresentation {
                    mime: content.mime,
                    data: content.data,
                })
                .collect(),
            available,
            remember,
            status: wire::ClipboardStatus::Success,
        },
        ClipboardAnswer::Write(completion) => DelegatedReply::ClipboardWrite {
            status: crate::bridge::write_status(completion.status),
            remember: completion.remember,
        },
        status => DelegatedReply::ClipboardMime {
            contents: Vec::new(),
            available: Vec::new(),
            remember: false,
            status: match status {
                ClipboardAnswer::Unsupported => wire::ClipboardStatus::Unsupported,
                ClipboardAnswer::Busy => wire::ClipboardStatus::Busy,
                _ => wire::ClipboardStatus::IoError,
            },
        },
    }
}

pub(crate) fn write_status(status: crate::ClipboardWriteResult) -> wire::ClipboardStatus {
    use crate::ClipboardWriteResult as S;
    match status {
        S::Success => wire::ClipboardStatus::Success,
        S::Denied => wire::ClipboardStatus::Denied,
        S::Unsupported => wire::ClipboardStatus::Unsupported,
        S::Busy => wire::ClipboardStatus::Busy,
        S::InvalidData => wire::ClipboardStatus::InvalidData,
        S::IoError => wire::ClipboardStatus::IoError,
    }
}

#[derive(Debug, Default)]
pub(crate) struct Peer {
    pub expected_nonce: Option<String>,
    pub nonce: Option<String>,
    pub(crate) pastes: VecDeque<(String, Vec<crate::ClipboardContent>, Instant)>,
    pub(crate) next_id: u64,
    /// [`crate::PaneConfig::bridge_osc_input`].
    pub(crate) osc_input: bool,
    /// The origin reads envelopes framed as an APC ([`Message::ApcInput`]), which a pseudoconsole passes.
    pub(crate) apc_input: bool,
}
#[derive(Clone, Copy)]
pub(crate) struct PasteOptions<'a> {
    pub text_source: bool,
    pub selection: &'a str,
    pub allow_unsafe: bool,
    pub lazy: bool,
}
impl Peer {
    /// A peer that expects the config's launch nonce, only on a viewer of a capability-advertising attach endpoint.
    pub(crate) fn expecting(config: &crate::PaneConfig) -> Self {
        Self {
            expected_nonce: config.attach_bridge.clone(),
            osc_input: config.bridge_osc_input,
            ..Self::default()
        }
    }

    /// Takes the connection's handshake (its Hello, then whether the origin reads APC envelopes); whether the envelope
    /// is another message of the connection.
    fn handshake(&mut self, nonce: &str, message: &Message) -> bool {
        if self.expected_nonce.as_deref() != Some(nonce) {
            return false;
        }
        match message {
            Message::Hello => {
                self.pastes.clear();
                self.nonce = Some(nonce.to_owned());
                self.apc_input = false;
                false
            }
            _ if self.nonce.as_deref() != Some(nonce) => false,
            Message::ApcInput => {
                self.apc_input = true;
                false
            }
            _ => true,
        }
    }

    /// An envelope as the origin's input reads it: an APC when the origin takes one, else the OSC.
    fn encode(apc_input: bool, envelope: &Envelope) -> Vec<u8> {
        if apc_input {
            envelope.encode_apc()
        } else {
            envelope.encode()
        }
    }

    pub fn paste(
        &mut self,
        pane: &Pane,
        contents: &[crate::ClipboardContent],
        options: PasteOptions<'_>,
    ) -> Result<bool, crate::PaneError> {
        let PasteOptions {
            text_source,
            selection,
            allow_unsafe,
            lazy,
        } = options;
        let Some(nonce) = self.nonce.clone().filter(|_| self.apc_input || self.osc_input) else {
            return Ok(false);
        };
        self.next_id += 1;
        let id = self.next_id.to_string();
        let bridge = if lazy && !text_source {
            Some(wire::ClipboardPasteBridge {
                id: id.clone(),
                available: contents.iter().take(16).map(|part| part.mime.clone()).collect(),
            })
        } else {
            None
        };
        let envelope = Envelope {
            version: VERSION,
            nonce,
            message: Message::Paste(wire::ClipboardPaste {
                contents: if bridge.is_none() {
                    contents
                        .iter()
                        .map(|part| wire::ClipboardRepresentation {
                            mime: part.mime.clone(),
                            data: part.data.clone(),
                        })
                        .collect()
                } else {
                    Vec::new()
                },
                bridge,
                text_source,
                selection: selection.into(),
                allow_unsafe,
            }),
        };
        let bytes = Self::encode(self.apc_input, &envelope);
        if bytes.len() > ngterm_core::attach_bridge::LIMIT {
            return Err(crate::PaneError::Backpressure);
        }
        pane.write(&bytes)?;
        if lazy && !text_source {
            while self.pastes.len() >= 32 {
                self.pastes.pop_front();
            }
            self.pastes
                .push_back((id, contents.iter().take(16).cloned().collect(), Instant::now()));
        }
        Ok(true)
    }
    fn fetch(&mut self, bridge: &str, mimes: &[String], list: bool) -> DelegatedReply {
        let Some(index) = self.pastes.iter().position(|(id, _, _)| id == bridge) else {
            return DelegatedReply::Denied;
        };
        if self.pastes[index].2.elapsed() >= Duration::from_secs(30) {
            self.pastes.remove(index);
            return DelegatedReply::Denied;
        }
        let contents = &self.pastes[index].1;
        let available = if list {
            contents.iter().map(|part| part.mime.clone()).collect()
        } else {
            Vec::new()
        };
        let contents = mimes
            .iter()
            .take(4)
            .filter_map(|mime| contents.iter().find(|part| &part.mime == mime))
            .map(|part| wire::ClipboardRepresentation {
                mime: part.mime.clone(),
                data: part.data.clone(),
            })
            .collect();
        if !mimes.is_empty() {
            self.pastes.remove(index);
        }
        DelegatedReply::ClipboardMime {
            contents,
            available,
            remember: false,
            status: wire::ClipboardStatus::Success,
        }
    }
}

impl Worker {
    /// A role report the origin's attach client also sent through the bridge, as its title may not arrive: reported as
    /// that title, which the frontend reads as a role report.
    fn bridge_role(&self, report: String) -> bool {
        if LeadNotice::parse(&report).is_some() {
            self.shared.send(PaneEvent::Title(report));
            self.shared.wake();
        }
        true
    }

    pub(crate) fn bridge_envelope(&mut self, envelope: Envelope) -> bool {
        let mut peer = self
            .shared
            .bridge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !peer.handshake(&envelope.nonce, &envelope.message) {
            return true;
        }
        let apc_input = peer.apc_input;
        let (id, epoch, request) = match envelope.message {
            Message::Request { id, epoch, request } => (id, epoch, request),
            Message::Role(report) => {
                drop(peer);
                return self.bridge_role(report);
            }
            _ => return true,
        };
        let fetched = if let DelegatedRequest::ClipboardReadMime {
            bridge: Some(bridge),
            mimes,
            list,
            ..
        } = &request
        {
            Some(peer.fetch(bridge, mimes, *list))
        } else {
            None
        };
        drop(peer);
        let _open = fetched.is_none().then(|| OpenRequest::new(&self.shared));
        let reply = if let Some(reply) = fetched {
            reply
        } else {
            self.next_read += 1;
            let local_id = self.next_read;
            self.shared.send(delegated_event(local_id, &request));
            self.shared.wake();
            let Some(answer) = self.wait_answer(local_id) else {
                return false;
            };
            answer_reply(answer)
        };
        let mut envelope = Envelope {
            version: VERSION,
            nonce: envelope.nonce,
            message: Message::Reply { id, epoch, reply },
        };
        let bytes = Peer::encode(apc_input, &envelope);
        let status = if bytes.len() > ngterm_core::attach_bridge::LIMIT {
            Some(wire::ClipboardStatus::InvalidData)
        } else if self.shared.bridge_input(&bytes).is_err() {
            Some(wire::ClipboardStatus::Busy)
        } else {
            None
        };
        if let Some(status) = status {
            let reply = match request {
                DelegatedRequest::ClipboardWrite { .. } => DelegatedReply::ClipboardWrite {
                    status,
                    remember: false,
                },
                DelegatedRequest::ClipboardRead { .. } => DelegatedReply::Denied,
                DelegatedRequest::ClipboardReadMime { .. } => DelegatedReply::ClipboardMime {
                    contents: Vec::new(),
                    available: Vec::new(),
                    remember: false,
                    status,
                },
            };
            envelope.message = Message::Reply { id, epoch, reply };
            self.shared.push_reply(&Peer::encode(apc_input, &envelope));
        }
        true
    }
}
