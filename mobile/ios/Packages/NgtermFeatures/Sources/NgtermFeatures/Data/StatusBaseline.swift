import NgtermPlatform

/// The last agent status seen of each attached session, kept in a ``StatusBaselineStore`` across launches, for a
/// background refresh to tell a turn from a repeat.
///
/// The live streams' statuses are recorded as they come, a snapshot's (`changed: false`, after a reconnect) as much as
/// a change's. The origin's clear (`changed: true` with no status: `completed` reset as the user visits the session)
/// is recorded as none, so the next turn to the same status notifies again; a clear with `changed: false` (the stream
/// went away, or a snapshot with no status, which can't be told apart) keeps the last one. A refresh lists each
/// session's status and notifies only a turn to waiting (`blocked`) or done (`completed`) against this baseline, once:
/// snapshot statuses carry `changed: false`, so the stream's own rule would notify nothing. A detached session's status
/// is let go.
@preconcurrency
@MainActor
public final class StatusBaseline {
    private static let waiting = "blocked"
    private static let done = "completed"

    private let store: StatusBaselineStore
    private var statuses: [String: [String: String]]

    /// The baseline over `connections`, whose statuses it records from now on, kept in `store`.
    public init(connections: Connections, store: StatusBaselineStore) {
        self.store = store
        statuses = store.statuses
        connections.listen { [weak self] event in
            guard case .sessionStatus(let machineId, let sessionId, let status, let changed) = event else {
                return
            }
            let key = SessionKey(machineId: machineId, sessionId: sessionId)
            if let status {
                self?.record(key, status: status.status)
            } else if changed {
                self?.clear(key)
            }
        }
        connections.watchAttached { [weak self] attached in
            self?.keep(attached)
        }
    }

    /// The kind of notice a status `status` (its wire name) is: waiting for `blocked`, done for `completed`, none
    /// otherwise.
    public static func kind(of status: String?) -> NoticeKind? {
        switch status {
        case waiting:
            .waiting

        case done:
            .done

        default:
            nil
        }
    }

    /// The last status seen of `key`.
    public func status(of key: SessionKey) -> String? {
        statuses[key.machineId]?[key.sessionId]
    }

    /// Records `status` as the last seen of `key`, notifying nothing.
    public func record(_ key: SessionKey, status: String) {
        guard self.status(of: key) != status else {
            return
        }
        statuses[key.machineId, default: [:]][key.sessionId] = status
        save()
    }

    /// Forgets the last status of `key`: the origin cleared it, and whatever it turns to next is a turn.
    public func clear(_ key: SessionKey) {
        guard statuses[key.machineId]?.removeValue(forKey: key.sessionId) != nil else {
            return
        }
        if statuses[key.machineId]?.isEmpty == true {
            statuses[key.machineId] = nil
        }
        save()
    }

    /// The kind the agent of `key` turned, now listed with `status` (`nil`: none).
    ///
    /// Waiting or done when it is one and wasn't before, `nil` for a repeat or anything else. Records `status` either
    /// way, a listing with none as a clear.
    public func turn(_ key: SessionKey, to status: String?) -> NoticeKind? {
        guard let status else {
            clear(key)
            return nil
        }
        let before = self.status(of: key)
        record(key, status: status)
        return before == status ? nil : Self.kind(of: status)
    }

    // lets go of the statuses of the sessions no longer attached
    private func keep(_ attached: [String: Set<String>]) {
        var kept: [String: [String: String]] = [:]
        for (machineId, sessions) in statuses {
            let ids = attached[machineId] ?? []
            kept[machineId] = sessions.filter { ids.contains($0.key) }
        }
        kept = kept.filter { !$0.value.isEmpty }
        guard kept != statuses else {
            return
        }
        statuses = kept
        save()
    }

    private func save() {
        // a baseline the store can't write is kept for this run: a refresh in another may notify a turn again
        try? store.save(statuses)
    }

    deinit {
        // nothing to release: the connections hold only weak references back
    }
}
