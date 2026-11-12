import Foundation
import NgtermCore
import Observation

// where a session a notification's tap names stands, as the attach for it waits
private enum TapReadiness: Sendable {
    case attached
    case gone
    case listed
    case waiting
}

// The sessions of the machines: New Session, End Session, attach (a notification tap's too) and detach.
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
        // a name an earlier stream of the session said is not this one's; attaching an attached one keeps its stream
        let key = SessionKey(machineId: machineId, sessionId: sessionId)
        if attached[machineId]?.contains(sessionId) != true {
            streamNames[key] = nil
        }
        attaching[key, default: 0] += 1
        defer {
            attaching[key, default: 1] -= 1
            if attaching[key] == 0 {
                attaching[key] = nil
            }
        }
        try await core.attachSession(machineId: machineId, sessionId: sessionId)
        attached[machineId, default: []].insert(sessionId)
        renamed(machineId)
    }

    /// Whether an attach of `session` is under way.
    public func isAttaching(_ session: SessionKey) -> Bool {
        attaching[session] != nil
    }

    /// Attaches `sessionId` of `machineId` for a notification's tap once the machine is connected and lists it.
    ///
    /// After the app was started afresh for the tap (a background refresh's notice), nothing is attached yet and the
    /// machine is still connecting. One attached already is attached at once.
    ///
    /// The wait ends with nothing attached as soon as `wanted` no longer holds (the user went elsewhere), the machine's
    /// connect needs the user (a host key, a sign-in) or failed, its listing failed or doesn't have the session, the
    /// attach failed, or the caller's task was cancelled.
    ///
    /// - Returns: whether the session is attached.
    @preconcurrency
    public func attachWhenListed(
        machineId: String,
        sessionId: String,
        while wanted: @escaping @MainActor () -> Bool
    ) async -> Bool {
        let readiness = Observations { @MainActor [weak self] in
            guard let self, wanted() else {
                return TapReadiness.gone
            }
            return tapReadiness(machineId: machineId, sessionId: sessionId)
        }
        for await state in readiness {
            switch state {
            case .attached:
                return true

            case .gone:
                return false

            case .listed:
                return (try? await attach(machineId: machineId, sessionId: sessionId)) != nil

            case .waiting:
                guard !Task.isCancelled else {
                    return false
                }
            }
        }
        return false
    }

    private func tapReadiness(machineId: String, sessionId: String) -> TapReadiness {
        if attached[machineId]?.contains(sessionId) == true {
            return .attached
        }
        switch link(of: machineId) {
        case .connected:
            if let sessions = trees[machineId] {
                return sessions.contains { $0.id == sessionId } ? .listed : .gone
            }
            return treeFailures[machineId] == nil ? .waiting : .gone

        case .connecting, .reconnecting:
            return .waiting

        case .hostKeyChanged, .hostKeyUnknown, .offline, .signIn, .unreachable:
            return .gone
        }
    }

    /// Detaches `sessionId` of `machineId`: it goes on running there.
    public func detach(machineId: String, sessionId: String) {
        var left = attached[machineId] ?? []
        left.remove(sessionId)
        attached[machineId] = left.isEmpty ? nil : left
        streamNames[SessionKey(machineId: machineId, sessionId: sessionId)] = nil
        renamed(machineId)
        guarded { try core.detachSession(machineId: machineId, sessionId: sessionId) }
    }
}
