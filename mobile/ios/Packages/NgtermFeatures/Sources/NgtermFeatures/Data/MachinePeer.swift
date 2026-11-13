import NgtermCore

/// The app a machine runs, and what that app offers (mobile/android's `MachinePeer`).
///
/// The app is the one its last listing said (``RemoteSession/peer``), what it offers the core's
/// ``peerCapabilities(peer:)``. A machine with none known runs ngterm, which offers everything.
public struct MachinePeer: Hashable, Sendable {
    /// The app.
    public let kind: PeerKind
    /// What it offers, and what the user misses there.
    public let offers: PeerCapabilities

    /// Whether the machine runs another app than ngterm, which the screens say with a badge and its ``note``.
    public var isForeign: Bool {
        kind != .ngterm
    }

    /// What the user misses on it, as one sentence; `nil` for ngterm.
    public var note: String? {
        offers.note
    }

    /// Whether it has the agent chat view, its commands and uploads.
    public var offersChat: Bool {
        offers.agentChat
    }

    /// The app's name, as its badge says it: a product name, not a translated word.
    public var badge: String {
        switch kind {
        case .ngterm:
            "ngterm"

        case .agterm:
            "agterm"
        }
    }

    /// `kind`, offering `offers`.
    public init(kind: PeerKind, offers: PeerCapabilities) {
        self.kind = kind
        self.offers = offers
    }

    /// `kind`, offering what the core says it does.
    public init(kind: PeerKind) {
        self.init(kind: kind, offers: peerCapabilities(peer: kind))
    }
}
