import NgtermCore

/// An ask an attached session's origin handed over, waiting for this app's answer (mobile/android's `OpenAsk`).
public struct OpenAsk: Hashable, Sendable, Identifiable {
    /// The session it came from.
    public let key: SessionKey
    /// The ask.
    public let ask: RemoteAsk

    /// Its name among every session's asks.
    public var id: String {
        "\(key.machineId):\(key.sessionId):\(ask.id)"
    }

    /// The ask `ask` of the session `key`.
    public init(key: SessionKey, ask: RemoteAsk) {
        self.key = key
        self.ask = ask
    }
}
