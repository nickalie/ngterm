import NgtermPlatform

/// The last agent status seen of each attached session, kept in a ``StatusBaselineStore`` across launches, for a
/// background refresh to tell a turn from a repeat.
///
/// The live streams' statuses are recorded as they come, a snapshot's (`changed: false`, after a reconnect) as much as
/// a change's; a stream's clear (it went away) keeps the last one. A refresh lists each session's status and notifies
/// only a turn to waiting (`blocked`) or done (`completed`) against this baseline, once: snapshot statuses carry
/// `changed: false`, so the stream's own rule would notify nothing. A detached session's status is let go.
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
            if case .sessionStatus(let machineId, let sessionId, let status, _) = event, let status {
                self?.record(SessionKey(machineId: machineId, sessionId: sessionId), status: status.status)
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

    /// The kind the agent of `key` turned, now listed with `status`.
    ///
    /// Waiting or done when it is one and wasn't before, `nil` for a repeat or anything else. Records `status` either
    /// way.
    public func turn(_ key: SessionKey, to status: String?) -> NoticeKind? {
        guard let status else {
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
