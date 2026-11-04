import Foundation
import NgtermCore

// The sessions of the machines: New Session, End Session, attach and detach.
extension Connections {
    /// Creates a session on `machineId`, named `name` (blank for the machine's own name), and returns it once it is
    /// listed; the listing that has it arrives in ``trees``.
    ///
    /// - Throws: ``MobileError`` with the core's words when it could not.
    public func createSession(machineId: String, name: String) async throws -> RemoteSession {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return try await core.createSession(machineId: machineId, name: trimmed.isEmpty ? nil : trimmed)
    }

    /// Ends `sessionId` on `machineId`, with no undo, and lets go of it here when it was attached.
    ///
    /// - Throws: ``MobileError`` with the core's words when it could not.
    public func endSession(machineId: String, sessionId: String) async throws {
        try await core.endSession(machineId: machineId, sessionId: sessionId)
        if attached[machineId]?.contains(sessionId) == true {
            detach(machineId: machineId, sessionId: sessionId)
        }
    }

    /// Attaches `sessionId` of `machineId`, a session of its last listing; attaching an attached one changes nothing.
    ///
    /// - Throws: ``MobileError`` with the core's words when it could not.
    public func attach(machineId: String, sessionId: String) async throws {
        try await core.attachSession(machineId: machineId, sessionId: sessionId)
        attached[machineId, default: []].insert(sessionId)
    }

    /// Detaches `sessionId` of `machineId`: it goes on running there.
    public func detach(machineId: String, sessionId: String) {
        var left = attached[machineId] ?? []
        left.remove(sessionId)
        attached[machineId] = left.isEmpty ? nil : left
        guarded { try core.detachSession(machineId: machineId, sessionId: sessionId) }
    }
}
