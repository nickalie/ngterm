import NgtermCore

/// A pick an attached session's origin bound to it (`ngtermctl pick --session`), waiting for this app's answer
/// (mobile/android's `OpenPick`).
public struct OpenPick: Hashable, Sendable, Identifiable {
    /// The session it came from.
    public let key: SessionKey
    /// The pick.
    public let pick: RemotePick

    /// Its name among every session's picks.
    public var id: String {
        "\(key.machineId):\(key.sessionId):\(pick.id)"
    }

    /// The pick `pick` of the session `key`.
    public init(key: SessionKey, pick: RemotePick) {
        self.key = key
        self.pick = pick
    }
}
