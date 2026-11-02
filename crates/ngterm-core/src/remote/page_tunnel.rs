//! A loopback URL page carried to the machine presenting its session, an extension beyond upstream, which refuses such a
//! page (`OverlayHtmlError.remoteLoopback`): a page an origin opens at `http://127.0.0.1:PORT/` names a server on the
//! origin, so the presenter forwards a port of its own loopback to it over ssh and loads the page there.
//!
//! - The origin hands such a page over only to a presenter whose hello lists [`TUNNEL_KIND`]; any other keeps
//!   upstream's refusal.
//! - The presenter starts the forward as a page helper ([`super::replica_page::PageHelperLauncher`]): an ssh with
//!   `-L` whose remote command prints [`READY_LINE`] and then waits on its stdin. ssh binds the forward before the
//!   session runs that command, so the line arriving means the port answers; the page is placed only then, so it never
//!   loads before its server is reachable. Stopping the helper closes its stdin, which ends the forward.
//! - The page's address on the presenter is the forwarded one ([`TunnelPlan::local_url`]); what the origin reads back
//!   names its own address again ([`TunnelPlan::origin_address`]).
//! - The page loads in a browser store of its own, never the saved one: that store keys cookies by host alone, and
//!   every forwarded page's host is 127.0.0.1. The origin's commands that arrive before the page is placed apply once
//!   it is.
//! - ssh's connection sharing is off for the forward, which would otherwise belong to a shared master and outlive the
//!   page.
//! - The forward ending ends the page, as a file page's helper ending does.

use url::Url;

/// The capability a viewer's hello lists when it carries an origin's loopback pages over ssh. It names no frame body.
pub const TUNNEL_KIND: &str = "page.tunnel";

/// What the forward's remote command prints once the forward is up.
pub const READY_LINE: &str = "ngterm-tunnel-ready";

/// A loopback page's forward: where it reaches on the origin and where the presenter loads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelPlan {
    /// The host the origin serves the page on, as ssh's `-L` takes it (an IPv6 address in brackets).
    pub target_host: String,
    pub target_port: u16,
    /// The port of this machine's loopback the forward listens on.
    pub local_port: u16,
    /// The page's address on this machine.
    pub local_url: Url,
    /// The page's origin on the other machine, `scheme://host[:port]`.
    origin_base: String,
    /// The same on this machine.
    local_base: String,
}

impl TunnelPlan {
    /// The forward of `address`, a loopback URL on the origin, to `local_port` on this machine; `None` for an address
    /// that is no loopback web URL.
    #[must_use]
    pub fn new(address: &str, local_port: u16) -> Option<Self> {
        if !crate::html::is_loopback(address) {
            return None;
        }
        let origin = Url::parse(address).ok()?;
        let target_host = origin.host_str()?.to_owned();
        let target_port = origin.port_or_known_default()?;
        let mut local_url = origin.clone();
        local_url.set_host(Some("127.0.0.1")).ok()?;
        local_url.set_port(Some(local_port)).ok()?;
        Some(Self {
            target_host,
            target_port,
            local_port,
            origin_base: origin.origin().ascii_serialization(),
            local_base: local_url.origin().ascii_serialization(),
            local_url,
        })
    }

    /// `address`, one this machine's page shows, as the origin names it: the forwarded origin becomes the origin's own
    /// again, any other address stays as it is.
    #[must_use]
    pub fn origin_address(&self, address: &str) -> String {
        match address.strip_prefix(&self.local_base) {
            Some(rest) if rest.is_empty() || rest.starts_with(['/', '?', '#']) => format!("{}{rest}", self.origin_base),
            _ => address.to_owned(),
        }
    }
}

/// A free port of this machine's loopback, for a forward to listen on. Another program may take it before ssh binds
/// it; the forward then fails and the page is refused.
///
/// # Errors
///
/// No port could be bound.
pub fn free_local_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loopback_page_is_forwarded_to_the_same_path_on_this_machine() {
        let plan = TunnelPlan::new("http://127.0.0.1:47231/board?run=3#top", 50123).unwrap();
        assert_eq!(plan.target_host, "127.0.0.1");
        assert_eq!(plan.target_port, 47231);
        assert_eq!(plan.local_url.as_str(), "http://127.0.0.1:50123/board?run=3#top");
        let plan = TunnelPlan::new("http://localhost/", 50123).unwrap();
        assert_eq!((plan.target_host.as_str(), plan.target_port), ("localhost", 80));
        let plan = TunnelPlan::new("https://[::1]:8443/x", 50123).unwrap();
        assert_eq!((plan.target_host.as_str(), plan.target_port), ("[::1]", 8443));
        assert_eq!(plan.local_url.as_str(), "https://127.0.0.1:50123/x");
    }

    #[test]
    fn only_a_loopback_web_url_is_forwarded() {
        for address in [
            "http://example.com/",
            "file:///tmp/a.html",
            "http://127.0.0.2/",
            "not a url",
        ] {
            assert_eq!(TunnelPlan::new(address, 50123), None, "{address}");
        }
    }

    #[test]
    fn the_origin_reads_its_own_address_back() {
        let plan = TunnelPlan::new("http://localhost:5173/", 50123).unwrap();
        assert_eq!(
            plan.origin_address("http://127.0.0.1:50123/runs/2?x=1"),
            "http://localhost:5173/runs/2?x=1"
        );
        assert_eq!(plan.origin_address("http://127.0.0.1:50123"), "http://localhost:5173");
        assert_eq!(plan.origin_address("https://example.com/"), "https://example.com/");
        assert_eq!(
            plan.origin_address("http://127.0.0.1:501234/"),
            "http://127.0.0.1:501234/",
            "another port that starts with the same digits is not the forward"
        );
    }

    #[test]
    fn a_free_port_is_on_the_loopback() {
        let port = free_local_port().unwrap();
        assert_ne!(port, 0);
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    }
}
