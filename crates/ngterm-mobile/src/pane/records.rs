//! What a shown pane tells the app and the answers it takes back, as `UniFFI` records.

use ngterm_core::live::lead::LeadRole;
use ngterm_term::ClipboardLocation;

/// Which pane an event is about.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct PaneRef {
    pub machine_id: String,
    pub session_id: String,
    /// The pane's identity, as the session's layout names it (an upper-case UUID).
    pub pane_id: String,
}

/// A pane's role at its session on the machine: only the leader's grid sizes the machine's pty, so a follower draws
/// output laid out for another client's grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PaneRole {
    /// No client leads (the leader left); the pane attaches again on its own to take the lead if nobody else has.
    Unowned,
    Leader,
    Follower,
}

impl From<LeadRole> for PaneRole {
    fn from(role: LeadRole) -> Self {
        match role {
            LeadRole::Unowned => Self::Unowned,
            LeadRole::Leader => Self::Leader,
            LeadRole::Follower => Self::Follower,
        }
    }
}

/// How a pane's attach ended.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PaneEnd {
    /// The link went away (the connection or the channel): the pane stays covered and attaches again into a fresh
    /// terminal on its own, once its machine is connected.
    Lost { reason: String },
    /// The program exited: its status (`None` when unknown or when a signal ended it) and the signal that ended it.
    /// The terminal keeps the last screen; the pane attaches again by itself only when the machine respawned its
    /// shell.
    Exited { code: Option<u32>, signal: Option<String> },
    /// The session is gone from its machine: final, nothing attaches it again.
    Gone,
}

/// Which clipboard a program reads or writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PaneClipboard {
    Standard,
    Selection,
    Primary,
}

impl From<ClipboardLocation> for PaneClipboard {
    fn from(location: ClipboardLocation) -> Self {
        match location {
            ClipboardLocation::Standard => Self::Standard,
            ClipboardLocation::Selection => Self::Selection,
            ClipboardLocation::Primary => Self::Primary,
        }
    }
}

/// One representation of a clipboard value: its MIME type and bytes (text as UTF-8 under `text/plain`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ClipboardPart {
    pub mime: String,
    pub data: Vec<u8>,
}

impl From<ngterm_term::ClipboardContent> for ClipboardPart {
    fn from(content: ngterm_term::ClipboardContent) -> Self {
        Self {
            mime: content.mime,
            data: content.data,
        }
    }
}

impl From<ClipboardPart> for ngterm_term::ClipboardContent {
    fn from(part: ClipboardPart) -> Self {
        Self {
            mime: part.mime,
            data: part.data,
        }
    }
}

/// The user's answer to a program's clipboard read.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ClipboardReply {
    /// Hand over `contents`: each requested type the clipboard holds, under the name the request gave it (none: it
    /// holds none of them), and `available`, the types it holds, when the request asked for them. `remember` keeps the
    /// answer for the program in this terminal: the request's next reads are allowed unasked (honoured only when the
    /// request could be remembered).
    Allow {
        contents: Vec<ClipboardPart>,
        available: Vec<String>,
        remember: bool,
    },
    /// Refuse: the program reads an empty clipboard (OSC 52) or is told it was refused (the Kitty protocol).
    Deny,
}

/// How the app's write of a program's clipboard value ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ClipboardWriteStatus {
    /// The device's clipboard holds the value (or was cleared).
    Done,
    /// The user or the app's policy refused it.
    Denied,
    /// The device's clipboard keeps none of the value's types.
    Unsupported,
    /// The device's clipboard refused it.
    Failed,
}
