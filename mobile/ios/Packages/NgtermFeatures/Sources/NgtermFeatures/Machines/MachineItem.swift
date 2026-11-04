import NgtermCore
import NgtermPlatform

/// A machine on the Machines list: the machine, its link, and its sessions once it has listed them.
public struct MachineItem: Hashable, Sendable, Identifiable {
    /// The machine.
    public let machine: StoredMachine
    /// Where its connection stands.
    public let link: Link
    /// Whether it has listed its sessions since it connected.
    public let isListed: Bool
    /// Its sessions as it last listed them; none before a listing.
    public let sessions: [RemoteSession]

    /// The machine's id.
    public var id: String {
        machine.id
    }

    /// `machine`, its `link`, and its `sessions` when it `isListed`.
    public init(machine: StoredMachine, link: Link, isListed: Bool, sessions: [RemoteSession]) {
        self.machine = machine
        self.link = link
        self.isListed = isListed
        self.sessions = sessions
    }
}
