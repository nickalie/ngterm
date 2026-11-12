//! Neutral attach clipboard types. No terminal engine, OS, callback or password crosses this seam.
use serde::{Deserialize, Serialize};

/// Clipboard paste contents and source, generated on the authoritative terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardPaste {
    /// Lazy attach paste; the outer terminal token never leaves its relay.
    pub bridge: Option<ClipboardPasteBridge>,
    pub contents: Vec<ClipboardRepresentation>,
    pub text_source: bool,
    pub selection: String,
    pub allow_unsafe: bool,
}

/// An opaque attach bridge and its advertised types; no eagerly fetched contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardPasteBridge {
    pub id: String,
    pub available: Vec<String>,
}

/// Binary-safe clipboard representation; zero entries means clear.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardRepresentation {
    pub mime: String,
    #[serde(with = "representation_data")]
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardStatus {
    Success,
    Denied,
    Unsupported,
    Busy,
    InvalidData,
    IoError,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DelegatedRequest {
    /// An OSC 52 read of `selection`: `c` standard, `s` selection, `p` primary.
    /// `list_only` never serves contents and requires no consent.
    ClipboardRead { selection: String, list_only: bool },
    /// A Kitty read/listing; `granted` is computed by the server from its terminal token grants.
    ClipboardReadMime {
        /// Present only for a validated one-time paste from this source client and epoch.
        bridge: Option<String>,
        selection: String,
        mimes: Vec<String>,
        list: bool,
        name: String,
        granted: bool,
        can_remember: bool,
    },
    /// A committed write; server-computed `granted` conveys token consent, and `kitty` selects Kitty policy.
    ClipboardWrite {
        selection: String,
        contents: Vec<ClipboardRepresentation>,
        name: String,
        granted: bool,
        can_remember: bool,
        kitty: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DelegatedReply {
    /// The clipboard's text; `None` when it holds none (the program reads an empty clipboard).
    Clipboard { text: Option<String> },
    /// MIME read/listing result; `status` is the native outcome and `remember` grants this read token.
    ClipboardMime {
        contents: Vec<ClipboardRepresentation>,
        available: Vec<String>,
        remember: bool,
        status: ClipboardStatus,
    },
    /// Native write completion; `status` controls the program ACK, `remember` grants this write token.
    ClipboardWrite { status: ClipboardStatus, remember: bool },
    /// Refused by policy or by the user.
    Denied,
}

/// Compact JSON bridge data while preserving the binary session protocol's Vec<u8> layout.
mod representation_data {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(data: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.serialize_str(&STANDARD.encode(data))
        } else {
            data.serialize(serializer)
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        if deserializer.is_human_readable() {
            STANDARD
                .decode(String::deserialize(deserializer)?)
                .map_err(serde::de::Error::custom)
        } else {
            Vec::<u8>::deserialize(deserializer)
        }
    }
}
