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
//! - The forward listens on the origin's own port when this machine has it free and no other forward holds it
//!   ([`forward_port`]), so the page sends a `Host` the origin's server takes for its own: a server guarding against
//!   DNS rebinding answers only `127.0.0.1:PORT` or `localhost:PORT`. Only a port taken here falls back to another
//!   one. The page loads at 127.0.0.1 even when the origin named `localhost`: the forward listens on IPv4 alone, and
//!   `localhost` here may reach a server of this machine's own on `[::1]`.
//! - The page loads in a browser store of its own, never the saved one: that store keys cookies by host alone, and
//!   every forwarded page's host is 127.0.0.1. The origin's commands that arrive before the page is placed apply once
//!   it is.
//! - ssh's connection sharing is off for the forward, which would otherwise belong to a shared master and outlive the
//!   page.
//! - The forward ending ends the page, as a file page's helper ending does.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};

use socket2::{Domain, Protocol, Socket, Type};
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

/// The port of this machine's loopback a forward to the origin's `target_port` listens on: that same port when no
/// server of this machine's holds it ([`port_free`]) and no forward of `held` (this machine's other forwards, some of
/// them not bound by ssh yet) has it, so the page's `Host` names the origin's own port; else any free one. Another
/// program may take it before ssh binds it; the forward then fails and the page is refused.
///
/// # Errors
///
/// No port could be bound.
pub fn forward_port(target_port: u16, held: impl IntoIterator<Item = u16>) -> std::io::Result<u16> {
    if target_port != 0 && !held.into_iter().any(|port| port == target_port) && port_free(target_port) {
        return Ok(target_port);
    }
    Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?.local_addr()?.port())
}

/// Whether no server of this machine's listens on `port` where a connection to 127.0.0.1 would reach it. Binding
/// 127.0.0.1 alone does not tell: macOS lets it sit beside a wildcard listener under the `SO_REUSEADDR` std and ssh
/// set, and Windows beside one even without, so ssh would bind it too and take that server's loopback traffic. A bind
/// of the wildcard itself collides with such a listener everywhere: `0.0.0.0`, and `[::]` in dual mode, which a
/// dual-stack server holds (Windows' default is IPv6 alone, which never answers 127.0.0.1). The probes bind and never
/// listen, so no firewall asks about a wildcard listener. A machine without IPv6 has no dual-stack server.
fn port_free(port: u16) -> bool {
    let v4 = [Ipv4Addr::LOCALHOST, Ipv4Addr::UNSPECIFIED]
        .into_iter()
        .all(|address| bind_alone(SocketAddr::from((address, port))).is_ok());
    v4 && match bind_alone(SocketAddr::from((Ipv6Addr::UNSPECIFIED, port))) {
        Ok(()) => true,
        Err(error) => !matches!(
            error.kind(),
            std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
        ),
    }
}

/// Binds a TCP socket to `address` without listening, an IPv6 one in dual mode, with the `SO_REUSEADDR` std sets on
/// Unix, so a connection a closed forward left in `TIME_WAIT` does not hold its port.
fn bind_alone(address: SocketAddr) -> std::io::Result<()> {
    let socket = Socket::new(Domain::for_address(address), Type::STREAM, Some(Protocol::TCP))?;
    #[cfg(unix)]
    socket.set_reuse_address(true)?;
    if address.is_ipv6() {
        socket.set_only_v6(false)?;
    }
    socket.bind(&address.into())
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

    /// A port nothing holds right now.
    fn unused_port() -> u16 {
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[test]
    fn a_forward_takes_the_origins_port_when_it_is_free_here() {
        let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let busy = taken.local_addr().unwrap().port();
        let port = forward_port(busy, []).unwrap();
        assert_ne!(port, 0);
        assert_ne!(port, busy, "a port taken here falls back to another");
        drop(taken);
        assert_eq!(forward_port(busy, []).unwrap(), busy, "a free port is the origin's own");
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok());
    }

    /// A server listening on a wildcard of `address`'s family, an IPv6 one in dual mode as Node and most dev servers
    /// open it; `None` on a machine without that family.
    fn wildcard_server(address: SocketAddr) -> Option<(Socket, u16)> {
        let socket = Socket::new(Domain::for_address(address), Type::STREAM, Some(Protocol::TCP)).ok()?;
        if address.is_ipv6() {
            socket.set_only_v6(false).ok()?;
        }
        socket.bind(&address.into()).ok()?;
        socket.listen(1).ok()?;
        let port = socket.local_addr().ok()?.as_socket()?.port();
        Some((socket, port))
    }

    #[test]
    fn a_server_listening_on_a_wildcard_keeps_its_port() {
        let (any_v4, port) = wildcard_server(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))).unwrap();
        assert_ne!(forward_port(port, []).unwrap(), port, "0.0.0.0 answers 127.0.0.1 too");
        drop(any_v4);
        if let Some((_any, port)) = wildcard_server(SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))) {
            assert_ne!(
                forward_port(port, []).unwrap(),
                port,
                "a dual-stack [::] answers 127.0.0.1 too"
            );
        }
    }

    #[test]
    fn a_port_another_forward_holds_is_not_taken_again() {
        let free = unused_port();
        let port = forward_port(free, [free]).unwrap();
        assert_ne!(port, free, "a forward whose ssh has not bound yet still holds its port");
        assert_ne!(port, 0);
    }

    #[test]
    fn a_forwarded_page_sends_the_host_its_origin_serves() {
        let plan = TunnelPlan::new("http://127.0.0.1:41163/", 41163).unwrap();
        assert_eq!(plan.local_url.as_str(), "http://127.0.0.1:41163/");
        assert_eq!(
            plan.origin_address("http://127.0.0.1:41163/runs"),
            "http://127.0.0.1:41163/runs"
        );
        let plan = TunnelPlan::new("http://localhost:41163/", 41163).unwrap();
        assert_eq!(
            plan.local_url.as_str(),
            "http://127.0.0.1:41163/",
            "loaded on the IPv4 forward, never a server of this machine's own on [::1]"
        );
        assert_eq!(
            plan.origin_address("http://127.0.0.1:41163/"),
            "http://localhost:41163/"
        );
    }

    #[test]
    fn a_zero_target_port_asks_for_a_usable_ephemeral_port() {
        let port = forward_port(0, []).unwrap();
        assert_ne!(port, 0);
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok());
    }

    #[test]
    fn query_and_fragment_navigation_read_back_on_the_origin() {
        let plan = TunnelPlan::new("https://localhost/", 50123).unwrap();
        for (local, origin) in [
            ("https://127.0.0.1:50123?q=x", "https://localhost?q=x"),
            ("https://127.0.0.1:50123#top", "https://localhost#top"),
            ("https://127.0.0.1:50123.evil/x", "https://127.0.0.1:50123.evil/x"),
        ] {
            assert_eq!(plan.origin_address(local), origin);
        }
    }

    /// An IPv6-only holder of `[::]` fails the dual-mode probe, though it never answers 127.0.0.1; whether the port
    /// comes back the origin's own or another is not pinned, only that the forward can bind it. Bound and never
    /// listening, so nothing accepts connections on a wildcard while the test runs.
    #[test]
    fn a_port_an_ipv6_only_socket_holds_still_gets_a_bindable_forward() {
        let Ok(socket) = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP)) else {
            return; // IPv6 is optional on the host.
        };
        socket.set_only_v6(true).unwrap();
        if socket
            .bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)).into())
            .is_err()
        {
            return;
        }
        let port = socket.local_addr().unwrap().as_socket().unwrap().port();
        let forward = forward_port(port, []).unwrap();
        assert_ne!(forward, 0);
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, forward)).is_ok());
    }
}
