import NgtermPlatform

// What an alert says as a notification (mobile/android's `Notifications.describe`), and what its tap and actions carry
// back to the app.
extension Alert {
    /// The key of the machine's id in a notice's info.
    public static let machineInfo = "machine"
    /// The key of the session's id in a notice's info.
    public static let sessionInfo = "session"
    /// The key of the pane's id in a notice's info.
    public static let paneInfo = "pane"
    /// The key of the ask's id in an ask's notice's info.
    public static let askInfo = "ask"
    /// What an ask's action's id starts with; the button's id follows.
    public static let answerPrefix = "answer:"

    /// The notification.
    public var notice: Notice {
        let (title, subtitle, body) = words
        return Notice(
            id: id,
            threadId: threadId,
            title: title,
            body: body,
            subtitle: subtitle,
            info: info,
            actions: actions
        )
    }

    // a session's alerts group together, a machine's own apart
    private var threadId: String {
        let place = target
        guard let sessionId = place.sessionId else {
            return "machine:\(place.machineId)"
        }
        return "session:\(place.machineId):\(sessionId)"
    }

    private var info: [String: String] {
        let place = target
        var carried = [Self.machineInfo: place.machineId]
        carried[Self.sessionInfo] = place.sessionId
        carried[Self.paneInfo] = place.paneId
        if case .ask(let open, _, _) = self {
            carried[Self.askInfo] = open.ask.id
        }
        return carried
    }

    // the title, the line under it and the text
    private var words: (String, String, String) {
        switch self {
        case .ask(let open, let subject, let delivery):
            let message = open.ask.message.flatMap { AskText.isBlank($0) ? nil : $0 }
            // one answered in the app says so, as its text or under its message
            let hint = delivery == .openApp ? String(localized: "Tap to answer in ngterm.") : nil
            let text = [message, hint].compactMap(\.self).joined(separator: "\n")
            return (open.ask.title, String(localized: "\(subject.session) on \(subject.machine)"), text)

        case .lost(_, let machine, let reason):
            return (String(localized: "Lost \(machine)"), "", Self.lostText(machine, reason))

        case .notice(_, let subject, let notice):
            let title = AskText.isBlank(notice.title) ? subject.session : notice.title
            return (title, subject.session, notice.body)

        case .status(let subject, let kind, let context, _):
            let title =
                kind == .waiting
                ? String(localized: "\(subject.session) is waiting for you")
                : String(localized: "\(subject.session) finished")
            let line = context.flatMap { AskText.isBlank($0) ? nil : $0 }
            return (title, "", line ?? String(localized: "On \(subject.machine)"))
        }
    }

    // an ask's buttons, as its delivery says: a reply per answering button of an input ask
    private var actions: [NoticeAction] {
        guard case .ask(let open, _, let delivery) = self, delivery != .openApp else {
            return []
        }
        let ask = open.ask
        let hint = ask.input?.placeholder ?? String(localized: "Your answer")
        return ask.buttons.map { button in
            let replies = delivery == .reply && button.id != ask.cancelId
            return NoticeAction(
                id: Self.answerPrefix + button.id,
                title: button.label,
                isDestructive: button.id == ask.destructiveId,
                replyPlaceholder: replies ? hint : nil
            )
        }
    }

    private static func lostText(_ machine: String, _ reason: LostReason) -> String {
        switch reason {
        case .hostKeyChanged:
            String(
                localized: """
                    \(machine)'s host key changed, so ngterm stopped connecting. Open the machine to review it.
                    """
            )

        case .unreachable(let words):
            words
        }
    }
}
