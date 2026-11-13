import NgtermCoreBindings

/// What a machine running `peer` offers, and what the user misses there as one sentence (crates/ngterm-mobile,
/// peer.rs's `peer_capabilities`); `nil` reads as ngterm, which offers everything.
public func peerCapabilities(peer: PeerKind?) -> PeerCapabilities {
    NgtermCoreBindings.peerCapabilities(peer: peer)
}
