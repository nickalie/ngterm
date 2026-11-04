import NgtermCore

/// An attached session, by its machine and its id there.
public struct SessionKey: Hashable, Sendable {
    /// The machine's id in the app's store.
    public let machineId: String
    /// The session's id on the machine.
    public let sessionId: String

    /// The session `sessionId` of the machine `machineId`.
    public init(machineId: String, sessionId: String) {
        self.machineId = machineId
        self.sessionId = sessionId
    }

    /// The session `pane` belongs to.
    public init(_ pane: PaneRef) {
        self.init(machineId: pane.machineId, sessionId: pane.sessionId)
    }
}
