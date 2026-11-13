//! Which app a machine runs, as the apps see it: another ngterm, or upstream agterm, whose sessions the core lists,
//! attaches and presents too, with fewer features ([`ngterm_core::remote::peer`]). The core learns it from each listing
//! ([`crate::sessions`]) and forms every later command for it, and refuses the chat calls on agterm, which has no agent
//! chat view. It reports the app on each [`RemoteSession::peer`](crate::sessions::RemoteSession) and what it lacks
//! through [`peer_capabilities`], for the apps to show: the Android app badges such a machine and its sessions with the
//! app's name and note, and turns their chat view off; the iOS app reads neither yet.

use ngterm_core::remote::peer::{self, Capabilities, RemotePeer};

/// The app on a machine's far side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PeerKind {
    Ngterm,
    Agterm,
}

impl From<RemotePeer> for PeerKind {
    fn from(peer: RemotePeer) -> Self {
        match peer {
            RemotePeer::Ngterm => Self::Ngterm,
            RemotePeer::Agterm => Self::Agterm,
        }
    }
}

impl From<PeerKind> for RemotePeer {
    fn from(peer: PeerKind) -> Self {
        match peer {
            PeerKind::Ngterm => Self::Ngterm,
            PeerKind::Agterm => Self::Agterm,
        }
    }
}

/// What a peer offers beyond the protocol both apps speak, and the line an app shows about it.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is one feature a peer offers or lacks, independent of the others"
)]
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PeerCapabilities {
    /// The agent chat view, its slash commands and uploads; the core refuses those calls without it.
    pub agent_chat: bool,
    /// The Kitty clipboard's paste bridge on a pane's attach.
    pub clipboard_bridge: bool,
    /// Renames made on the machine follow onto its sessions here.
    pub follows_renames: bool,
    /// Pages the machine opens as companions later are handed over.
    pub page_companions: bool,
    /// What the user misses, as one sentence; `None` for ngterm.
    pub note: Option<String>,
}

/// What `peer` offers; `None` reads as ngterm.
#[uniffi::export]
#[must_use]
pub fn peer_capabilities(peer: Option<PeerKind>) -> PeerCapabilities {
    let peer: RemotePeer = peer.map_or(RemotePeer::Ngterm, RemotePeer::from);
    let offered = Capabilities::of(peer);
    PeerCapabilities {
        agent_chat: offered.agent_chat,
        clipboard_bridge: offered.clipboard_bridge,
        follows_renames: offered.follows_renames,
        page_companions: offered.page_companions,
        note: peer::limits_note(peer),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agterm_machine_offers_no_chat_and_says_what_it_lacks() {
        let agterm = peer_capabilities(Some(PeerKind::Agterm));
        assert!(!agterm.agent_chat && !agterm.clipboard_bridge && !agterm.follows_renames && !agterm.page_companions);
        assert!(
            agterm
                .note
                .as_deref()
                .is_some_and(|note| note.contains("agent chat view"))
        );
        let ngterm = peer_capabilities(Some(PeerKind::Ngterm));
        assert!(ngterm.agent_chat && ngterm.note.is_none());
        assert_eq!(peer_capabilities(None), ngterm);
        assert_eq!(RemotePeer::from(PeerKind::from(RemotePeer::Agterm)), RemotePeer::Agterm);
    }
}
