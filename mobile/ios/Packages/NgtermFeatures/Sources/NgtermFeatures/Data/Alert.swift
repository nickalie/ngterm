import NgtermCore
import NgtermPlatform

/// A notification the app posts (mobile/android's `Alert`); its ``id`` names it: a later alert with the same id
/// replaces it, and withdrawing the id takes it down.
public enum Alert: Hashable, Sendable {
    /// An open ask, out of sight, answered as `delivery` says.
    case ask(OpenAsk, about: AlertSubject, delivery: AskDelivery)
    /// A machine with attached sessions was lost for good.
    case lost(machineId: String, machine: String, reason: LostReason)
    /// A notification the agent of a session sent; `number` tells it apart from the session's others.
    case notice(number: UInt64, about: AlertSubject, notice: RemoteNotice)
    /// The agent of a session is waiting (``NoticeKind/waiting``) or done (``NoticeKind/done``); `context` says what it
    /// works on.
    case status(about: AlertSubject, kind: NoticeKind, context: String?, pane: String?)

    /// Its name.
    public var id: String {
        switch self {
        case .ask(let open, _, _):
            Self.askId(open.key, askId: open.ask.id)

        case .lost(let machineId, _, _):
            Self.lostId(machineId)

        case .notice(let number, let subject, _):
            "notice:\(subject.key.machineId):\(subject.key.sessionId):\(number)"

        case .status(let subject, _, _, _):
            Self.statusId(subject.key)
        }
    }

    /// Where its tap leads.
    public var target: OpenTarget {
        switch self {
        case .ask(let open, _, _):
            OpenTarget(machineId: open.key.machineId, sessionId: open.key.sessionId, paneId: open.ask.pane)

        case .lost(let machineId, _, _):
            OpenTarget(machineId: machineId)

        case .notice(_, let subject, let notice):
            OpenTarget(machineId: subject.key.machineId, sessionId: subject.key.sessionId, paneId: notice.pane)

        case .status(let subject, _, _, let pane):
            OpenTarget(machineId: subject.key.machineId, sessionId: subject.key.sessionId, paneId: pane)
        }
    }

    /// Whether the posted alert `id`, about the session `key` (none for a machine's), shows while the app is in the
    /// foreground with the sessions `onScreen` on screen.
    ///
    /// Every one does but a status or an ask of a session on screen, which came just before it was: its view says it
    /// now. An agent's own notice shows beside its session too, as mobile/android's does.
    public static func showsInForeground(id: String, key: SessionKey?, onScreen: Set<SessionKey>) -> Bool {
        guard let key, onScreen.contains(key) else {
            return true
        }
        return id != statusId(key) && !id.hasPrefix(askId(key, askId: ""))
    }

    /// The id of the agent status alert of `key`: one per session, the latest replacing the one before.
    public static func statusId(_ key: SessionKey) -> String {
        "status:\(key.machineId):\(key.sessionId)"
    }

    /// The id of the lost alert of `machineId`.
    public static func lostId(_ machineId: String) -> String {
        "lost:\(machineId)"
    }

    /// The id of the alert of the ask `askId` of `key`.
    public static func askId(_ key: SessionKey, askId: String) -> String {
        "ask:\(key.machineId):\(key.sessionId):\(askId)"
    }
}
