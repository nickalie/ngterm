//! Private clipboard envelopes on designated attach panes. The attach relay suppresses this
//! reserved OSC on every program-output path before injecting its own envelopes.
use crate::clipboard_wire::{ClipboardPaste, DelegatedReply, DelegatedRequest};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const VARIABLE: &str = "NGTERM_ATTACH_BRIDGE";
pub const PREFIX: &[u8] = b"\x1b]777;ngterm-attach;";
pub const BODY_PREFIX: &[u8] = b"777;ngterm-attach;";
pub const LIMIT: usize = 64 * 1024 * 1024;
/// Raw contents budget after two base64 layers, reserving space for metadata and MIME names.
pub const DATA_LIMIT: usize = (LIMIT - 64 * 1024) * 9 / 16;

/// Whether this endpoint can safely launch the private attach protocol: an ngterm origin's that advertises it, never
/// an agterm origin's.
#[must_use]
pub fn supported(endpoint: &ngterm_proto::ControlLiveEndpoint) -> bool {
    endpoint.attach_bridge == Some(VERSION) && crate::remote::peer::Capabilities::of(endpoint.peer()).clipboard_bridge
}

#[must_use]
pub fn variable(nonce: &str) -> String {
    format!("{VERSION}:{nonce}")
}

/// The launch nonce only when the endpoint advertises this protocol.
#[must_use]
pub fn launch_nonce<'a>(
    endpoint: &ngterm_proto::ControlLiveEndpoint,
    lead: &'a crate::live::lead::LeadAttachment,
) -> Option<&'a str> {
    supported(endpoint).then_some(lead.nonce.as_str())
}

/// Validate the explicit attach-bridge launch contract before touching a terminal or session.
#[must_use]
pub fn parse_variable(value: &str) -> Option<&str> {
    let (version, nonce) = value.split_once(':')?;
    (version.parse::<u32>().ok()? == VERSION
        && !nonce.is_empty()
        && nonce.len() <= 128
        && nonce
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    .then_some(nonce)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u32,
    pub nonce: String,
    pub message: Message,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    Hello,
    Request {
        id: u64,
        epoch: u64,
        request: DelegatedRequest,
    },
    Reply {
        id: u64,
        epoch: u64,
        reply: DelegatedReply,
    },
    Paste(ClipboardPaste),
}
impl Envelope {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let Ok(json) = serde_json::to_vec(self) else {
            return Vec::new();
        };
        [PREFIX, STANDARD.encode(json).as_bytes(), b"\x1b\\"].concat()
    }
    #[must_use]
    pub fn decode(body: &[u8]) -> Option<Self> {
        if body.len() > LIMIT {
            return None;
        }
        let bytes = STANDARD.decode(body.strip_prefix(BODY_PREFIX)?).ok()?;
        let envelope: Self = serde_json::from_slice(&bytes).ok()?;
        (envelope.version == VERSION && !envelope.nonce.is_empty() && envelope.nonce.len() <= 128).then_some(envelope)
    }
}

/// Incremental extraction before the terminal parser. Non-envelopes retain every byte;
/// only the reserved prefix is buffered, and an oversized envelope is discarded to ST/BEL.
#[derive(Debug, Default)]
pub struct Decoder {
    held: Vec<u8>,
    in_frame: bool,
    overflow: bool,
    escape: bool,
}
#[derive(Debug)]
pub enum Part {
    Bytes(Vec<u8>),
    Envelope(Envelope),
}

#[derive(Debug, Default)]
enum Suppression {
    #[default]
    Ground,
    Prefix,
    Frame {
        escape: bool,
    },
}

/// Remove reserved envelopes even inside other VT strings. Forward ESC immediately:
/// it may already be the dispatch byte of a delegated clipboard operation. On recognizing
/// the reserved prefix, CAN cancels that ESC before any envelope metadata reaches a viewer.
#[derive(Debug, Default)]
pub struct Suppressor {
    held: Vec<u8>,
    state: Suppression,
}
impl Suppressor {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        for &byte in bytes {
            match &mut self.state {
                Suppression::Ground => {
                    output.push(byte);
                    if byte == 0x1b {
                        self.state = Suppression::Prefix;
                    }
                }
                Suppression::Prefix => self.prefix_byte(byte, &mut output),
                Suppression::Frame { escape } => {
                    if byte == 7 || (*escape && byte == b'\\') {
                        self.state = Suppression::Ground;
                    } else {
                        *escape = byte == 0x1b;
                    }
                }
            }
        }
        output
    }
    fn prefix_byte(&mut self, byte: u8, output: &mut Vec<u8>) {
        self.held.push(byte);
        if !PREFIX[1..].starts_with(&self.held) {
            output.append(&mut self.held);
            self.state = if byte == 0x1b {
                Suppression::Prefix
            } else {
                Suppression::Ground
            };
        } else if self.held == PREFIX[1..] {
            self.held.clear();
            output.push(0x18);
            self.state = Suppression::Frame { escape: false };
        }
    }
}

impl Decoder {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Part> {
        let mut parts = Vec::new();
        let mut plain = Vec::new();
        for &byte in bytes {
            if self.in_frame {
                self.frame_byte(byte, &mut parts);
                continue;
            }
            self.held.push(byte);
            while !PREFIX.starts_with(&self.held) {
                plain.push(self.held.remove(0));
            }
            if self.held == PREFIX {
                if !plain.is_empty() {
                    parts.push(Part::Bytes(std::mem::take(&mut plain)));
                }
                self.in_frame = true;
            }
        }
        if !plain.is_empty() {
            parts.push(Part::Bytes(plain));
        }
        parts
    }
    fn frame_byte(&mut self, byte: u8, parts: &mut Vec<Part>) {
        if byte == 7 || (self.escape && byte == b'\\') {
            self.finish_frame(parts);
            return;
        }
        self.escape = byte == 0x1b;
        if self.held.len() < LIMIT {
            self.held.push(byte);
        } else {
            self.overflow = true;
        }
    }
    fn finish_frame(&mut self, parts: &mut Vec<Part>) {
        if !self.overflow {
            if self.escape {
                self.held.pop();
            }
            if let Some(envelope) = Envelope::decode(&self.held[2..]) {
                parts.push(Part::Envelope(envelope));
            }
        }
        self.held.clear();
        self.in_frame = false;
        self.overflow = false;
        self.escape = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_clipboards_use_compact_json_and_fit_the_bridge_budget() {
        let data: Vec<_> = (0..=255_u8).cycle().take(14 * 1024 * 1024).collect();
        let representation = crate::clipboard_wire::ClipboardRepresentation {
            mime: "image/png".into(),
            data,
        };
        let small = crate::clipboard_wire::ClipboardRepresentation {
            mime: "image/png".into(),
            data: vec![0, 255],
        };
        assert_eq!(serde_json::to_value(&small).unwrap()["data"], "AP8=");
        let envelope = Envelope {
            version: VERSION,
            nonce: "connection".into(),
            message: Message::Paste(ClipboardPaste {
                bridge: None,
                contents: vec![representation],
                text_source: false,
                selection: "c".into(),
                allow_unsafe: false,
            }),
        };
        let encoded = envelope.encode();
        assert!(encoded.len() < LIMIT);
        assert_eq!(Envelope::decode(&encoded[2..encoded.len() - 2]), Some(envelope));
    }

    #[test]
    fn every_chunk_boundary_preserves_plain_bytes_and_separates_envelopes() {
        let envelope = Envelope {
            version: VERSION,
            nonce: "connection".into(),
            message: Message::Hello,
        };
        let bytes = [b"before\x1b[31m".as_slice(), &envelope.encode(), b"after\x1b[0m"].concat();
        for split in 0..=bytes.len() {
            let mut decoder = Decoder::default();
            let parts: Vec<_> = decoder
                .feed(&bytes[..split])
                .into_iter()
                .chain(decoder.feed(&bytes[split..]))
                .collect();
            let mut plain = Vec::new();
            let mut envelopes = Vec::new();
            for part in parts {
                match part {
                    Part::Bytes(bytes) => plain.extend(bytes),
                    Part::Envelope(value) => envelopes.push(value),
                }
            }
            assert_eq!(plain, b"before\x1b[31mafter\x1b[0m");
            assert_eq!(envelopes.as_slice(), std::slice::from_ref(&envelope));
        }
    }

    #[test]
    fn suppression_cancels_reserved_envelopes_inside_any_string_without_delaying_escape_dispatch() {
        let envelope = Envelope {
            version: VERSION,
            nonce: "forged".into(),
            message: Message::Hello,
        }
        .encode();
        for prefix in [b"plain".as_slice(), b"\x1b_opaque", b"\x1bPopaque", b"\x1b]0;title"] {
            let bytes = [prefix, &envelope, b"after"].concat();
            for split in 0..=bytes.len() {
                let mut filter = Suppressor::default();
                let output = [filter.feed(&bytes[..split]), filter.feed(&bytes[split..])].concat();
                assert_eq!(output, [prefix, b"\x1b\x18after"].concat());
            }
        }
        let mut filter = Suppressor::default();
        assert_eq!(
            filter.feed(b"\x1b"),
            b"\x1b",
            "ESC may dispatch a preceding clipboard OSC immediately"
        );
        assert_eq!(filter.feed(b"\\"), b"\\");
        let bytes = b"\x1b]777;ngterm-other\x1b\\\x1b\x1b[31mtext\x1b[0m";
        let mut filter = Suppressor::default();
        assert_eq!(filter.feed(bytes), bytes);
    }

    #[test]
    fn invalid_and_incompatible_reserved_envelopes_are_discarded_without_becoming_text() {
        for body in [b"not-base64!".as_slice(), b"e30=", b"/w=="] {
            let bytes = [PREFIX, body, b"\x07after"].concat();
            let mut decoder = Decoder::default();
            assert!(matches!(decoder.feed(&bytes).as_slice(), [Part::Bytes(bytes)] if bytes == b"after"));
        }
        let mut decoder = Decoder::default();
        let unsupported = Envelope {
            version: VERSION + 1,
            nonce: "connection".into(),
            message: Message::Hello,
        };
        assert!(decoder.feed(&unsupported.encode()).is_empty());
    }
}
