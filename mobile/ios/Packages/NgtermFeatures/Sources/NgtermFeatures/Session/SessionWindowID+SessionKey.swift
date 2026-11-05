extension SessionWindowID {
    /// The session as the routers know it.
    public var key: SessionKey {
        SessionKey(machineId: machineID, sessionId: sessionID)
    }

    /// The window value of the session `key`.
    public init(_ key: SessionKey) {
        self.init(machineID: key.machineId, sessionID: key.sessionId)
    }
}
