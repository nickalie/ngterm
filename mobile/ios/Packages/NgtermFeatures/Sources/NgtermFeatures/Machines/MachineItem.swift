import NgtermCore
import NgtermPlatform

/// A machine on the Machines list: the machine, its link, its sessions once it has listed them, and the app it runs
/// when a listing said.
public struct MachineItem: Hashable, Sendable, Identifiable {
    /// The machine.
    public let machine: StoredMachine
    /// Where its connection stands.
    public let link: Link
    /// Whether it has listed its sessions since it connected.
    public let isListed: Bool
    /// Its sessions as it last listed them; none before a listing.
    public let sessions: [RemoteSession]
    /// The app it runs, as a listing said; `nil`: ngterm, or not said.
    public let peer: MachinePeer?

    /// The machine's id.
    public var id: String {
        machine.id
    }

    /// `machine`, its `link`, its `sessions` when it `isListed`, and the app it runs (`peer`).
    public init(
        machine: StoredMachine,
        link: Link,
        isListed: Bool,
        sessions: [RemoteSession],
        peer: MachinePeer? = nil
    ) {
        self.machine = machine
        self.link = link
        self.isListed = isListed
        self.sessions = sessions
        self.peer = peer
    }
}
