//! The app on the far side of a remote session: another ngterm, or upstream agterm (an extension beyond upstream,
//! whose viewers attach only agterm). An agterm origin speaks the protocol this build's remote layer was ported from,
//! under its own names: `agtermctl zmx tree|present|list`, its bundled zmx for each pane's attach, zmx's attach
//! variables and role titles. Only those names differ, so a viewer forms each remote command for the peer that answers
//! it, and the presentation stream negotiates the rest in its hello. Everything agterm runs stays on the agterm machine:
//! a viewer, whatever its OS (a Windows viewer's panes go through [`super::viewer_run`], the mobile core through its own
//! ssh channels), only sends it command strings over ssh.
//!
//! Which peer a host is:
//! - a destination may pin it, `agterm:HOST` or `ngterm:HOST` ([`destination`]): the prefix stays in the row's host, so
//!   every later command and a relaunch reach the same app, and ssh is given only `HOST`;
//! - otherwise the listing finds it: the far side's `ngtermctl live tree` when it answers, else its `agtermctl zmx tree`,
//!   in one ssh call ([`super::ssh::tree_tail`]), whose agterm answer starts with [`AGTERM_ANSWER`]. What it found is
//!   stamped on the endpoint ([`ngterm_proto::ControlLiveEndpoint::peer`]) and learned for the host ([`learn`]).
//!
//! A row attached from a host keeps the peer its endpoint was stamped with: every later command for that row, and its
//! relaunch, go to that app, whichever one a later listing of the host finds. Only a command for no bound row (a new
//! session asked of a host from the picker) follows what the host's last listing found ([`peer_of`]).
//!
//! An agterm origin lacks what is this build's own ([`Capabilities`]), and the frontends say so where it shows.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

pub use ngterm_proto::RemotePeer;
use uuid::Uuid;

use crate::live::lead::LeadAttachment;

/// agterm's own names for what a viewer runs there (upstream's `RemoteSession.swift`, `CommandPath.swift`,
/// `ZmxSupport.swift` and `ZmxLead.swift`). An agterm peer's wire constants: nothing of this build's own runs under
/// them.
pub mod agterm {
    /// The CLI an agterm origin answers with.
    pub const CLI: &str = "agtermctl";
    /// Its command group for live sessions, this build's `live`.
    pub const LIVE_GROUP: &str = "zmx";
    /// Where its CLI is installed, appended to the bare PATH sshd runs a command with (upstream's `cliPathPrefix`).
    pub const PATH_DIRECTORIES: &str = "/usr/local/bin:/opt/homebrew/bin";
    /// The prefix of its pane daemons' names, followed by 32 lower-case hex digits.
    pub const DAEMON_PREFIX: &str = "agterm-";
    /// The attach variables its zmx reads (upstream's `ZmxSupport`, `attachCommand`).
    pub const SESSION_VARIABLE: &str = "ZMX_SESSION";
    pub const SESSION_PREFIX_VARIABLE: &str = "ZMX_SESSION_PREFIX";
    pub const NO_DETACH_KEY_VARIABLE: &str = "ZMX_NO_DETACH_KEY";
    pub const DIRECTORY_VARIABLE: &str = "ZMX_DIR";
    /// Its explicit-leadership variables (upstream's `ZmxLeadAttachment`).
    pub const NONCE_VARIABLE: &str = "ZMX_MANAGED";
    pub const CLAIM_VARIABLE: &str = "ZMX_MANAGED_CLAIM";
    /// The title prefix its zmx client reports a pane's role under (upstream's `ZmxLeadNotice.prefix`).
    pub const ROLE_TITLE_PREFIX: &str = "zmx-role;";
}

/// The line the listing prints ahead of an agterm origin's answer, so the viewer knows which app answered.
pub const AGTERM_ANSWER: &str = "ngterm-peer: agterm";

/// The text a destination pins `peer` with, `peer:`.
fn pin_prefix(peer: RemotePeer) -> String {
    format!("{}:", peer.name())
}

/// The peer a destination pins, if any, and what ssh is given: `agterm:HOST` → (`Agterm`, `HOST`). Only these two
/// prefixes are read; an ssh destination never starts with either (an IPv6 literal holds no `g`, `t`, `r` or `m`).
#[must_use]
pub fn destination(host: &str) -> (Option<RemotePeer>, &str) {
    [RemotePeer::Agterm, RemotePeer::Ngterm]
        .into_iter()
        .find_map(|peer| {
            host.strip_prefix(pin_prefix(peer).as_str())
                .map(|rest| (Some(peer), rest))
        })
        .unwrap_or((None, host))
}

/// What ssh is given for `host`: the destination without its pin.
#[must_use]
pub fn ssh_destination(host: &str) -> &str {
    destination(host).1
}

/// `host` pinned to `peer`, as a listing that must reach that app is sent: unchanged when it pins one already.
#[must_use]
pub fn pinned(host: &str, peer: RemotePeer) -> String {
    match destination(host) {
        (Some(_), _) => host.to_owned(),
        (None, rest) => format!("{}{rest}", pin_prefix(peer)),
    }
}

/// What was learned about each host in this process.
static LEARNED: LazyLock<Mutex<HashMap<String, RemotePeer>>> = LazyLock::new(Mutex::default);

/// The app a command for no bound row of `host` goes to: its pin, else what its last listing found, else ngterm. A bound
/// row's commands go to its endpoint's peer instead.
#[must_use]
pub fn peer_of(host: &str) -> RemotePeer {
    if let (Some(pinned), _) = destination(host) {
        return pinned;
    }
    LEARNED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(host)
        .copied()
        .unwrap_or_default()
}

/// Records which app answered `host`'s listing.
pub fn learn(host: &str, peer: RemotePeer) {
    LEARNED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(host.to_owned(), peer);
}

/// A listing's output without its [`AGTERM_ANSWER`] line, and the peer that line names (ngterm when there is none).
#[must_use]
pub fn split_answer(stdout: &str) -> (RemotePeer, &str) {
    let trimmed = stdout.trim_start_matches(['\r', '\n']);
    match trimmed.strip_prefix(AGTERM_ANSWER) {
        Some(rest) if rest.is_empty() || rest.starts_with(['\r', '\n']) => (RemotePeer::Agterm, rest),
        _ => (RemotePeer::Ngterm, stdout),
    }
}

/// The CLI `peer` answers with.
#[must_use]
pub fn cli(peer: RemotePeer) -> &'static str {
    match peer {
        RemotePeer::Ngterm => super::tree::REMOTE_CLI,
        RemotePeer::Agterm => agterm::CLI,
    }
}

/// `arguments` in `peer`'s CLI words: agterm's live-session group is `zmx`.
#[must_use]
pub fn cli_words<S: AsRef<str>>(peer: RemotePeer, arguments: &[S]) -> Vec<String> {
    arguments
        .iter()
        .enumerate()
        .map(|(index, word)| match (peer, index, word.as_ref()) {
            (RemotePeer::Agterm, 0, "live") => agterm::LIVE_GROUP.to_owned(),
            (_, _, word) => word.to_owned(),
        })
        .collect()
}

/// Whether `name` is one of `peer`'s pane daemons: its prefix and 32 lower-case hex digits.
#[must_use]
pub fn is_daemon_name(peer: RemotePeer, name: &str) -> bool {
    match peer {
        RemotePeer::Ngterm => crate::live::is_daemon_name(name),
        RemotePeer::Agterm => name.strip_prefix(agterm::DAEMON_PREFIX).is_some_and(is_hex_identity),
    }
}

fn is_hex_identity(body: &str) -> bool {
    body.len() == 32
        && body
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The daemon `peer` names its pane `identity` after.
#[must_use]
pub fn daemon_name(peer: RemotePeer, identity: Uuid) -> String {
    match peer {
        RemotePeer::Ngterm => crate::live::daemon_name(identity),
        RemotePeer::Agterm => format!("{}{}", agterm::DAEMON_PREFIX, identity.simple()),
    }
}

/// The origin pane identity an origin daemon's name encodes, whichever app named it; `None` for any other name.
#[must_use]
pub fn origin_pane_identity(name: &str) -> Option<Uuid> {
    [RemotePeer::Ngterm, RemotePeer::Agterm]
        .into_iter()
        .find(|peer| is_daemon_name(*peer, name))
        .and_then(|_| name.rsplit('-').next())
        .and_then(|hex| Uuid::try_parse(hex).ok())
}

/// The attach variables of `peer`'s attach client, in the order an attach line sets them: the session and its prefix
/// cleared, the detach key passed to the program, the namespace directory.
#[must_use]
pub fn attach_environment(peer: RemotePeer, directory: &str) -> [(&'static str, String); 4] {
    let (session, prefix, no_detach, dir) = match peer {
        RemotePeer::Ngterm => (
            crate::live::SESSION_VARIABLE,
            crate::live::SESSION_PREFIX_VARIABLE,
            crate::live::NO_DETACH_KEY_VARIABLE,
            crate::live::DIRECTORY_VARIABLE,
        ),
        RemotePeer::Agterm => (
            agterm::SESSION_VARIABLE,
            agterm::SESSION_PREFIX_VARIABLE,
            agterm::NO_DETACH_KEY_VARIABLE,
            agterm::DIRECTORY_VARIABLE,
        ),
    };
    [
        (session, String::new()),
        (prefix, String::new()),
        (no_detach, "1".to_owned()),
        (dir, directory.to_owned()),
    ]
}

/// `lead`'s variables under `peer`'s names, sorted by name.
#[must_use]
pub fn lead_environment(peer: RemotePeer, lead: &LeadAttachment) -> Vec<(String, String)> {
    match peer {
        RemotePeer::Ngterm => lead.environment().into_iter().collect(),
        RemotePeer::Agterm => {
            let mut environment = vec![(agterm::NONCE_VARIABLE.to_owned(), lead.nonce.clone())];
            if lead.claim {
                environment.push((agterm::CLAIM_VARIABLE.to_owned(), "1".to_owned()));
            }
            environment.sort();
            environment
        }
    }
}

/// What a peer offers beyond the protocol both apps speak. A viewer leaves out, and says it left out, what its peer
/// lacks.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is one feature a peer offers or lacks, independent of the others"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// The agent chat view: a pane's agent transcript, its slash-command catalog and file uploads to the origin
    /// (`ngtermctl agent chat|commands|upload`).
    pub agent_chat: bool,
    /// The Kitty clipboard's paste bridge on a pane's attach (the endpoint's `attachBridge`).
    pub clipboard_bridge: bool,
    /// Renames made on the origin follow onto the attached row (the stream's `name` frame).
    pub follows_renames: bool,
    /// A page the origin opens as a companion is handed to the viewer (the stream's `companion.page` frame).
    pub page_companions: bool,
}

impl Capabilities {
    /// What `peer` offers.
    #[must_use]
    pub fn of(peer: RemotePeer) -> Self {
        let own = peer == RemotePeer::Ngterm;
        Self {
            agent_chat: own,
            clipboard_bridge: own,
            follows_renames: own,
            page_companions: own,
        }
    }

    /// What a user misses, each as a short phrase, in a fixed order: empty for ngterm.
    #[must_use]
    pub fn missing(self) -> Vec<&'static str> {
        [
            (self.agent_chat, "the agent chat view"),
            (self.clipboard_bridge, "Kitty clipboard paste"),
            (self.follows_renames, "renames made there"),
            (self.page_companions, "page companions opened there later"),
        ]
        .into_iter()
        .filter_map(|(offered, phrase)| (!offered).then_some(phrase))
        .collect()
    }
}

/// What a user misses with `peer`, as one phrase: `None` for ngterm.
#[must_use]
pub fn missing_phrase(peer: RemotePeer) -> Option<String> {
    let missing = Capabilities::of(peer).missing();
    let (last, rest) = missing.split_last()?;
    Some(if rest.is_empty() {
        (*last).to_owned()
    } else {
        format!("{} and {last}", rest.join(", "))
    })
}

/// The one line a frontend shows about a row attached from `peer`: `None` for ngterm, else the app and what it lacks.
#[must_use]
pub fn limits_note(peer: RemotePeer) -> Option<String> {
    missing_phrase(peer).map(|missing| format!("Attached from {}: {missing} are not available", peer.name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_names_the_peer_and_leaves_ssh_the_destination() {
        assert_eq!(destination("agterm:studio"), (Some(RemotePeer::Agterm), "studio"));
        assert_eq!(destination("ngterm:me@box"), (Some(RemotePeer::Ngterm), "me@box"));
        assert_eq!(destination("studio.local"), (None, "studio.local"));
        assert_eq!(destination("fe80::1"), (None, "fe80::1"));
        assert_eq!(ssh_destination("agterm:me@studio"), "me@studio");
        assert_eq!(pinned("me@studio", RemotePeer::Ngterm), "ngterm:me@studio");
        assert_eq!(pinned("studio", RemotePeer::Agterm), "agterm:studio");
        assert_eq!(pinned("agterm:studio", RemotePeer::Ngterm), "agterm:studio");
    }

    #[test]
    fn a_pin_wins_over_what_a_listing_learned() {
        learn("peer-test-pinned", RemotePeer::Agterm);
        assert_eq!(peer_of("peer-test-pinned"), RemotePeer::Agterm);
        assert_eq!(peer_of("ngterm:peer-test-pinned"), RemotePeer::Ngterm);
        assert_eq!(peer_of("peer-test-never-listed"), RemotePeer::Ngterm);
        learn("peer-test-pinned", RemotePeer::Ngterm);
        assert_eq!(peer_of("peer-test-pinned"), RemotePeer::Ngterm);
        assert_eq!(peer_of("agterm:peer-test-pinned"), RemotePeer::Agterm);
    }

    #[test]
    fn only_a_whole_marker_line_says_agterm() {
        assert_eq!(
            split_answer("ngterm-peer: agterm\n{\"ok\":true}"),
            (RemotePeer::Agterm, "\n{\"ok\":true}")
        );
        assert_eq!(split_answer("ngterm-peer: agterm"), (RemotePeer::Agterm, ""));
        assert_eq!(split_answer("{\"ok\":true}"), (RemotePeer::Ngterm, "{\"ok\":true}"));
        assert_eq!(
            split_answer("ngterm-peer: agtermx\n{}"),
            (RemotePeer::Ngterm, "ngterm-peer: agtermx\n{}")
        );
    }

    #[test]
    fn agterm_runs_its_own_cli_and_live_group() {
        assert_eq!(cli(RemotePeer::Ngterm), "ngtermctl");
        assert_eq!(cli(RemotePeer::Agterm), "agtermctl");
        assert_eq!(cli_words(RemotePeer::Agterm, &["live", "list"]), ["zmx", "list"]);
        assert_eq!(
            cli_words(RemotePeer::Agterm, &["session", "new", "--name", "live"]),
            ["session", "new", "--name", "live"]
        );
        assert_eq!(cli_words(RemotePeer::Ngterm, &["live", "list"]), ["live", "list"]);
    }

    #[test]
    fn each_peer_knows_only_its_own_daemon_names_and_both_decode() {
        let identity = Uuid::from_u128(0x42);
        let ngterm = crate::live::daemon_name(identity);
        let agterm = format!("agterm-{}", identity.simple());
        assert_eq!(daemon_name(RemotePeer::Ngterm, identity), ngterm);
        assert_eq!(daemon_name(RemotePeer::Agterm, identity), agterm);
        assert!(is_daemon_name(RemotePeer::Ngterm, &ngterm));
        assert!(!is_daemon_name(RemotePeer::Ngterm, &agterm));
        assert!(is_daemon_name(RemotePeer::Agterm, &agterm));
        assert!(!is_daemon_name(RemotePeer::Agterm, &ngterm));
        assert!(!is_daemon_name(RemotePeer::Agterm, "agterm-notes"));
        assert!(!is_daemon_name(
            RemotePeer::Agterm,
            &format!("agterm-{}", identity.simple()).to_uppercase()
        ));
        assert_eq!(origin_pane_identity(&ngterm), Some(identity));
        assert_eq!(origin_pane_identity(&agterm), Some(identity));
        assert_eq!(origin_pane_identity("agterm-notes"), None);
    }

    #[test]
    fn agterm_attaches_under_zmx_names() {
        let names: Vec<&str> = attach_environment(RemotePeer::Agterm, "/tmp/agterm-zmx-1")
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(
            names,
            ["ZMX_SESSION", "ZMX_SESSION_PREFIX", "ZMX_NO_DETACH_KEY", "ZMX_DIR"]
        );
        let lead = LeadAttachment {
            nonce: "n1".to_owned(),
            claim: true,
        };
        assert_eq!(
            lead_environment(RemotePeer::Agterm, &lead),
            [
                ("ZMX_MANAGED".to_owned(), "n1".to_owned()),
                ("ZMX_MANAGED_CLAIM".to_owned(), "1".to_owned())
            ]
        );
        let follower = LeadAttachment {
            nonce: "n2".to_owned(),
            claim: false,
        };
        assert_eq!(
            lead_environment(RemotePeer::Agterm, &follower),
            [("ZMX_MANAGED".to_owned(), "n2".to_owned())]
        );
        assert_eq!(
            lead_environment(RemotePeer::Ngterm, &lead),
            lead.environment().into_iter().collect::<Vec<_>>()
        );
    }

    #[test]
    fn agterm_lacks_this_builds_own_features_and_the_note_names_them() {
        assert_eq!(
            Capabilities::of(RemotePeer::Ngterm),
            Capabilities {
                agent_chat: true,
                clipboard_bridge: true,
                follows_renames: true,
                page_companions: true
            }
        );
        let agterm = Capabilities::of(RemotePeer::Agterm);
        assert!(!agterm.agent_chat && !agterm.clipboard_bridge && !agterm.follows_renames && !agterm.page_companions);
        assert_eq!(limits_note(RemotePeer::Ngterm), None);
        assert_eq!(
            limits_note(RemotePeer::Agterm).as_deref(),
            Some(
                "Attached from agterm: the agent chat view, Kitty clipboard paste, renames made there and page \
                 companions opened there later are not available"
            )
        );
    }
}
