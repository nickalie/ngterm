/// Where a notification's tap leads (mobile/android's `OpenTarget`): a session and its pane, or with ``sessionId``
/// `nil` only the machine.
public struct OpenTarget: Hashable, Sendable {
    /// The machine.
    public let machineId: String
    /// The session; `nil` for the machine alone.
    public let sessionId: String?
    /// The pane to select; `nil` for the one the session has.
    public let paneId: String?

    /// The target `machineId`, and its session `sessionId` and pane `paneId` when given.
    public init(machineId: String, sessionId: String? = nil, paneId: String? = nil) {
        self.machineId = machineId
        self.sessionId = sessionId
        self.paneId = paneId
    }
}
