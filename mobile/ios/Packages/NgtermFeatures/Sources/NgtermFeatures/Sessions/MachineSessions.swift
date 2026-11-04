import NgtermCore
import NgtermPlatform

/// What a machine's sessions screen knows: the machine, its link, its sessions and what is under way on them
/// (mobile/android's `MachineSessions`).
public struct MachineSessions: Hashable, Sendable {
    /// The machine.
    public var machine: StoredMachine
    /// Where its connection stands.
    public var link: Link
    /// Whether the machine has listed its sessions since it connected.
    public var isListed: Bool
    /// The last listing: none before one.
    public var sessions: [RemoteSession]
    /// Why the last listing failed, until one lands.
    public var failure: String?
    /// The sessions this app has attached, by id.
    public var attached: Set<String>
    /// Whether the device has no network.
    public var isOffline: Bool
    /// Whether a pull to refresh is under way.
    public var isRefreshing: Bool
    /// The sessions whose End is under way, by id.
    public var ending: Set<String>

    /// The screen's facts: `machine` with its `link`, its `sessions` when it `isListed`, why the last listing
    /// `failure`d, what is `attached`, whether the device `isOffline`, a refresh is under way, and which sessions are
    /// `ending`.
    public init(
        machine: StoredMachine,
        link: Link,
        isListed: Bool,
        sessions: [RemoteSession],
        failure: String?,
        attached: Set<String>,
        isOffline: Bool,
        isRefreshing: Bool = false,
        ending: Set<String> = []
    ) {
        self.machine = machine
        self.link = link
        self.isListed = isListed
        self.sessions = sessions
        self.failure = failure
        self.attached = attached
        self.isOffline = isOffline
        self.isRefreshing = isRefreshing
        self.ending = ending
    }
}
