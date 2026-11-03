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

/// The user's answer to a program's clipboard read.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ClipboardReply {
    /// Hand over the clipboard's text (`None`: it holds none).
    Allow { text: Option<String> },
    /// Refuse: the program reads an empty clipboard.
    Deny,
}
