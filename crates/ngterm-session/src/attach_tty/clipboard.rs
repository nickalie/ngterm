//! Native OSC 5522 packets, matching Ghostty's `terminal/kitty/clipboard_response.zig`
//! and `clipboard_command.zig` at `LIBGHOSTTY_VT_REV`. Passwords stay inside the relay.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::fmt::Write as _;

use crate::protocol::{ClipboardRepresentation, ClipboardStatus, DelegatedRequest};

#[derive(Clone, PartialEq, Eq)]
pub struct Packet {
    pub operation: String,
    pub status: String,
    pub id: String,
    pub mime: String,
    pub data: Vec<u8>,
    pub token: Vec<u8>,
    pub primary: bool,
}

impl std::fmt::Debug for Packet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Packet")
            .field("operation", &self.operation)
            .field("status", &self.status)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Packet {
    #[must_use]
    pub fn parse(body: &[u8]) -> Option<Self> {
        let body = body.strip_prefix(b"5522;")?;
        let mut fields = body.splitn(2, |byte| *byte == b';');
        let metadata = fields.next()?;
        let mut result = Self {
            operation: String::new(),
            status: String::new(),
            id: String::new(),
            mime: String::new(),
            data: STANDARD.decode(fields.next().unwrap_or_default()).ok()?,
            token: Vec::new(),
            primary: false,
        };
        for field in metadata.split(|byte| *byte == b':') {
            let (key, value) = field.split_once_byte(b'=')?;
            match key {
                b"type" => result.operation = std::str::from_utf8(value).ok()?.into(),
                b"status" => result.status = std::str::from_utf8(value).ok()?.into(),
                b"id" => result.id = std::str::from_utf8(value).ok()?.into(),
                b"mime" => result.mime = String::from_utf8(STANDARD.decode(value).ok()?).ok()?,
                b"pw" => result.token = STANDARD.decode(value).ok()?,
                b"loc" => result.primary = value == b"primary",
                _ => {}
            }
        }
        Some(result)
    }

    #[must_use]
    pub fn outcome(&self) -> Option<ClipboardStatus> {
        Some(match self.status.as_str() {
            "DONE" => ClipboardStatus::Success,
            "EPERM" => ClipboardStatus::Denied,
            "ENOSYS" => ClipboardStatus::Unsupported,
            "EBUSY" => ClipboardStatus::Busy,
            "EINVAL" => ClipboardStatus::InvalidData,
            "EIO" => ClipboardStatus::IoError,
            _ => return None,
        })
    }
}

trait SplitByte {
    fn split_once_byte(&self, byte: u8) -> Option<(&[u8], &[u8])>;
}
impl SplitByte for [u8] {
    fn split_once_byte(&self, byte: u8) -> Option<(&[u8], &[u8])> {
        let index = self.iter().position(|value| *value == byte)?;
        Some((&self[..index], &self[index + 1..]))
    }
}

#[must_use]
pub fn read_query(id: &str, selection: &str, mimes: &[String], list: bool, token: &[u8]) -> Vec<u8> {
    let mut metadata = format!("type=read:id={id}");
    if selection == "p" {
        metadata.push_str(":loc=primary");
    }
    if !token.is_empty() {
        let _ = write!(
            metadata,
            ":name={}:pw={}",
            STANDARD.encode("ngterm attach paste"),
            STANDARD.encode(token)
        );
    }
    let mut targets = mimes.iter().take(4).cloned().collect::<Vec<_>>();
    if list {
        targets.push(".".into());
    }
    format!("\x1b]5522;{metadata};{}\x1b\\", STANDARD.encode(targets.join(" "))).into_bytes()
}

#[must_use]
pub fn write_query(id: &str, request: &DelegatedRequest, token: &[u8]) -> Vec<u8> {
    let DelegatedRequest::ClipboardWrite {
        selection,
        contents,
        name,
        ..
    } = request
    else {
        return Vec::new();
    };
    let primary = if selection == "p" { ":loc=primary" } else { "" };
    let password = if token.is_empty() {
        String::new()
    } else {
        format!(":pw={}", STANDARD.encode(token))
    };
    let mut bytes = format!(
        "\x1b]5522;type=write:id={id}{primary}:name={}{password}\x1b\\",
        STANDARD.encode(name)
    )
    .into_bytes();
    for ClipboardRepresentation { mime, data } in contents {
        // Each packet's base64 is independently padded; splitting never changes the representation.
        if data.is_empty() {
            bytes.extend_from_slice(format!("\x1b]5522;type=wdata:mime={}\x1b\\", STANDARD.encode(mime)).as_bytes());
        }
        for chunk in data.chunks(4096) {
            bytes.extend_from_slice(
                format!(
                    "\x1b]5522;type=wdata:mime={};{}\x1b\\",
                    STANDARD.encode(mime),
                    STANDARD.encode(chunk)
                )
                .as_bytes(),
            );
        }
    }
    bytes.extend_from_slice(b"\x1b]5522;type=wdata\x1b\\");
    bytes
}

#[cfg(test)]
mod tests;
