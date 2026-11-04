import NgtermPlatform

/// A machine just removed, which ``MachinesModel/undoRemove()`` puts back where it was, with its pin.
public struct Removal: Hashable, Sendable {
    /// The machine as it was.
    public let machine: StoredMachine
    /// Where it was in the list.
    public let index: Int
    /// Its pinned host key, if it had one.
    public let pin: String?

    /// `machine`, removed from `index`, with its `pin`.
    public init(machine: StoredMachine, index: Int, pin: String?) {
        self.machine = machine
        self.index = index
        self.pin = pin
    }
}
