import Foundation

/// Where a machine's connection stands, as the app shows it.
///
/// A machine with none is ``offline``.
public enum Link: Hashable, Sendable {
    /// Connected.
    case connected
    /// A connect the app asked for is on its way.
    case connecting
    /// The machine offered `offered`, not the `pinned` key: the connection stopped until the pin is removed.
    case hostKeyChanged(pinned: String, offered: String)
    /// The machine has no pinned key and offered `fingerprint`: its connect waits for the user's answer.
    case hostKeyUnknown(fingerprint: String)
    /// Not connected and not trying: never connected, disconnected, or a host key the user declined.
    case offline
    /// Lost after being connected; the core tries again at `retryAt`.
    case reconnecting(retryAt: Date)
    /// The machine asks the user to sign in at `url` (from its `message`, as Tailscale SSH does for a `check`) before
    /// it lets the connect in; the machine's next state ends it, or a disconnect.
    ///
    /// `isReconnecting` when the wait is the core's reconnect of a lost connection, not a connect the app asked for:
    /// the machine is listed afresh once it is back, as after any reconnect.
    case signIn(message: String, url: String, isReconnecting: Bool)
    /// Down until the user tries again, for `reason` (the core's words).
    case unreachable(reason: String)

    /// Whether the core holds or is making a connection the user can end (Disconnect, Don't connect).
    public var isUnderWay: Bool {
        switch self {
        case .connected, .connecting, .reconnecting, .signIn:
            true

        case .hostKeyChanged, .hostKeyUnknown, .offline, .unreachable:
            false
        }
    }

    // a reconnect, or a sign-in its reconnect waits on: Connected afterwards lists the machine afresh
    internal var isReconnect: Bool {
        switch self {
        case .reconnecting:
            true

        case .signIn(_, _, let isReconnecting):
            isReconnecting

        case .connected, .connecting, .hostKeyChanged, .hostKeyUnknown, .offline, .unreachable:
            false
        }
    }
}
