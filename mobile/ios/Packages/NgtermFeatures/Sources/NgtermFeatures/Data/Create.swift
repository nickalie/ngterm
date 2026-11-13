/// A session create: a New session on a machine, or a Duplicate of one of its sessions.
public struct Create: Hashable, Sendable {
    /// The machine the session is created on.
    public let machineId: String
    /// The session a Duplicate copies; `nil` for a New session.
    public let source: String?

    /// A New session on `machineId`, or a Duplicate of its session `source`.
    public init(machineId: String, source: String? = nil) {
        self.machineId = machineId
        self.source = source
    }
}
