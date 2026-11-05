import NgtermCore
import Observation

/// The asks the attached sessions' origins handed over (mobile/android's `Asks`): open in ``open`` until they are
/// answered here or the core takes them down (answered elsewhere, ended on the origin, the stream gone), their session
/// is detached, or the core is about to suspend (``takeBack()``: the origin takes them back).
///
/// From the suspend until ``resume()`` no ask opens: one the core told before it stopped its streams has been taken
/// back with the rest.
///
/// Where one shows is the ask rule's: a sheet while its session is on screen (``onScreen``: its view shown in a window
/// in the foreground), otherwise a notification (``Alerts``).
@preconcurrency
@MainActor
@Observable
public final class Asks {
    /// The open asks, oldest first.
    public private(set) var open: [OpenAsk] = [] {
        didSet {
            if open != oldValue {
                changed()
            }
        }
    }

    /// The sessions shown right now, each in a window in the foreground.
    public private(set) var onScreen: Set<SessionKey> = [] {
        didSet {
            if onScreen != oldValue {
                changed()
            }
        }
    }

    @ObservationIgnored private let connections: Connections
    // between takeBack() and resume(): the origins have taken their asks back
    @ObservationIgnored private var isTakenBack = false
    @ObservationIgnored private var watchers: [@MainActor () -> Void] = []

    /// The asks over `connections`, whose events they take from now on.
    public init(connections: Connections) {
        self.connections = connections
        connections.listen { [weak self] event in
            self?.receive(event)
        }
        connections.watchAttached { [weak self] attached in
            self?.open.removeAll { attached[$0.key.machineId]?.contains($0.key.sessionId) != true }
        }
    }

    /// Hands `watcher` every change of ``open`` or ``onScreen`` from now on.
    @preconcurrency
    public func watch(_ watcher: @escaping @MainActor () -> Void) {
        watchers.append(watcher)
    }

    /// Whether the session `key` shows in a window in the foreground.
    public func showing(_ key: SessionKey, isShown: Bool) {
        if isShown {
            onScreen.insert(key)
        } else {
            onScreen.remove(key)
        }
    }

    /// The open ask `askId` of `key`, if it is still open.
    public func find(_ key: SessionKey, askId: String) -> OpenAsk? {
        open.first { $0.key == key && $0.ask.id == askId }
    }

    /// Answers the open ask `askId` of `key` with `reply`, once: an ask no longer open is left be (the origin took it
    /// back).
    ///
    /// - Returns: true when it was open.
    @discardableResult
    public func answer(_ key: SessionKey, askId: String, reply: AskReply) -> Bool {
        guard remove(key, askId: askId) else {
            return false
        }
        // a core that has stopped leaves nothing waiting for the answer
        try? connections.core.answerAsk(machineId: key.machineId, sessionId: key.sessionId, askId: askId, reply: reply)
        return true
    }

    /// The core is about to suspend, and the origins take their asks back (as upstream does when a presenter goes):
    /// every ask leaves ``open``, unanswered, and none opens until ``resume()``: an ask the core told before its
    /// suspend stopped the streams has no stream left to answer through, and nothing would take it down.
    public func takeBack() {
        isTakenBack = true
        open = []
    }

    /// The core is back: its streams hand the asks still waiting over again as they reconnect.
    public func resume() {
        isTakenBack = false
    }

    private func receive(_ event: MobileEvent) {
        switch event {
        case .ask(let machineId, let sessionId, let ask):
            guard !isTakenBack else {
                return
            }
            let key = SessionKey(machineId: machineId, sessionId: sessionId)
            // one with the same id again is the same ask, restated
            var next = open
            next.removeAll { $0.key == key && $0.ask.id == ask.id }
            next.append(OpenAsk(key: key, ask: ask))
            open = next

        case .askDismiss(let machineId, let sessionId, let askId):
            remove(SessionKey(machineId: machineId, sessionId: sessionId), askId: askId)

        default:
            break
        }
    }

    @discardableResult
    private func remove(_ key: SessionKey, askId: String) -> Bool {
        let before = open.count
        open.removeAll { $0.key == key && $0.ask.id == askId }
        return open.count != before
    }

    private func changed() {
        for watcher in watchers {
            watcher()
        }
    }

    deinit {
        // nothing to release: the connections hold only weak references back
    }
}
