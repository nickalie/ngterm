import NgtermCore
import Observation

/// The picks the attached sessions' origins handed over (mobile/android's `Picks`): open in ``open`` until they are
/// answered here or the core takes them down (answered elsewhere, their timeout ran out on the origin, the stream
/// gone), their session is detached, or the core is about to suspend (``takeBack()``: the origin takes them back).
///
/// One shows in the sheet over its session while that session is on screen and none of its asks is open
/// (``AskPrompt``); out of sight it waits, as desktop's picker waits in the window holding the session.
@preconcurrency
@MainActor
@Observable
public final class Picks {
    /// The open picks, oldest first.
    public private(set) var open: [OpenPick] = []

    @ObservationIgnored private let connections: Connections
    // between takeBack() and resume(): the origins have taken their picks back
    @ObservationIgnored private var isTakenBack = false

    /// The picks over `connections`, whose events they take from now on.
    public init(connections: Connections) {
        self.connections = connections
        connections.listen { [weak self] event in
            self?.receive(event)
        }
        connections.watchAttached { [weak self] attached in
            self?.open.removeAll { attached[$0.key.machineId]?.contains($0.key.sessionId) != true }
        }
    }

    /// The rows `pick` shows for `query`, as desktop's picker ranks them (``pickRows(items:query:allowCustom:)``).
    public static func rows(of pick: RemotePick, query: String) -> [PickRow] {
        pickRows(items: pick.items, query: query, allowCustom: pick.allowCustom)
    }

    /// The open pick `pickId` of `key`, if it is still open.
    public func find(_ key: SessionKey, pickId: String) -> OpenPick? {
        open.first { $0.key == key && $0.pick.id == pickId }
    }

    /// Answers the open pick `pickId` of `key` with `reply`, once: a pick no longer open is left be (the origin took it
    /// back).
    ///
    /// - Returns: true when it was open.
    @discardableResult
    public func answer(_ key: SessionKey, pickId: String, reply: PickReply) -> Bool {
        guard remove(key, pickId: pickId) else {
            return false
        }
        // a core that has stopped leaves nothing waiting for the answer
        try? connections.core.answerPick(
            machineId: key.machineId,
            sessionId: key.sessionId,
            pickId: pickId,
            reply: reply
        )
        return true
    }

    /// The core is about to suspend, and the origins take their picks back: every pick leaves ``open``, unanswered,
    /// and none opens until ``resume()``.
    public func takeBack() {
        isTakenBack = true
        open = []
    }

    /// The core is back: its streams hand the picks still waiting over again as they reconnect.
    public func resume() {
        isTakenBack = false
    }

    private func receive(_ event: MobileEvent) {
        switch event {
        case .pick(let machineId, let sessionId, let pick):
            guard !isTakenBack else {
                return
            }
            let key = SessionKey(machineId: machineId, sessionId: sessionId)
            // one with the same id again is the same pick, restated
            var next = open
            next.removeAll { $0.key == key && $0.pick.id == pick.id }
            next.append(OpenPick(key: key, pick: pick))
            open = next

        case .pickDismiss(let machineId, let sessionId, let pickId):
            remove(SessionKey(machineId: machineId, sessionId: sessionId), pickId: pickId)

        default:
            break
        }
    }

    @discardableResult
    private func remove(_ key: SessionKey, pickId: String) -> Bool {
        let before = open.count
        open.removeAll { $0.key == key && $0.pick.id == pickId }
        return open.count != before
    }

    deinit {
        // nothing to release: the connections hold only weak references back
    }
}
