/// A machine's attached sessions, in its listing's order (mobile/android's `AttachedGroup`).
public struct AttachedGroup: Hashable, Sendable {
    /// The machine's id in the app's store.
    public let machineId: String
    /// The machine's name.
    public let machine: String
    /// Its attached sessions.
    public let sessions: [AttachedSession]

    /// The `sessions` of the machine `machineId`, named `machine`.
    public init(machineId: String, machine: String, sessions: [AttachedSession]) {
        self.machineId = machineId
        self.machine = machine
        self.sessions = sessions
    }
}
