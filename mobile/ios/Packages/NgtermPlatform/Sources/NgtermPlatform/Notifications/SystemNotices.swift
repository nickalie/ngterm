import UserNotifications

/// The app's notifications on iOS, through `UNUserNotificationCenter` (mobile/android's `Notifications`).
///
/// A notice with actions gets a category of its own (its actions are its ask's buttons, which differ from ask to
/// ask), registered before the notice is added and dropped once it is withdrawn. Its actions run in the background and
/// ask for the device to be unlocked first; a reply action takes typed text. Posts and withdrawals are carried out in
/// the order they were made, so a notice withdrawn just after it was posted doesn't come back. Nothing shows while the
/// user has notifications off: the system drops it.
@preconcurrency
@MainActor
public final class SystemNotices: NoticeCenter {
    // the prefix of a notice's own category
    private static let categoryPrefix = "actions:"
    // the key of a notice's kind in its user info
    nonisolated private static let kindInfo = "ngterm.kind"
    // what a reply action's send button says
    private static let sendTitle = String(localized: "Send")

    private let center = UNUserNotificationCenter.current()
    // the categories of the notices up, by category id
    private var categories: [String: UNNotificationCategory] = [:]
    // the last post or withdrawal, which the next one waits for
    private var last: Task<Void, Never>?

    /// The system's notifications.
    public init() {
        // nothing to set up: the categories are registered as notices need them
    }

    /// What the user did with a notice, as the app takes it.
    ///
    /// A tap, or an action with what was typed in its field; the notice's info keeps the values that are text.
    nonisolated public static func response(_ response: UNNotificationResponse) -> NoticeResponse {
        let action = response.actionIdentifier
        return NoticeResponse(
            noticeId: response.notification.request.identifier,
            action: action == UNNotificationDefaultActionIdentifier ? nil : action,
            info: info(of: response.notification),
            text: (response as? UNTextInputNotificationResponse)?.userText
        )
    }

    /// What a notice carries (``Notice/info``): the values of its info that are text, without its kind.
    nonisolated public static func info(of notification: UNNotification) -> [String: String] {
        var info: [String: String] = [:]
        for case (let key as String, let value as String) in notification.request.content.userInfo {
            info[key] = key == kindInfo ? nil : value
        }
        return info
    }

    private static func category(of id: String) -> String {
        categoryPrefix + id
    }

    private static func systemAction(_ action: NoticeAction) -> UNNotificationAction {
        var options: UNNotificationActionOptions = [.authenticationRequired]
        if action.isDestructive {
            options.insert(.destructive)
        }
        guard let placeholder = action.replyPlaceholder else {
            return UNNotificationAction(identifier: action.id, title: action.title, options: options)
        }
        return UNTextInputNotificationAction(
            identifier: action.id,
            title: action.title,
            options: options,
            textInputButtonTitle: sendTitle,
            textInputPlaceholder: placeholder
        )
    }

    /// What `notice` says, carries and is, as the system takes it; its buttons' category is set apart.
    nonisolated internal static func content(of notice: Notice) -> UNMutableNotificationContent {
        let content = UNMutableNotificationContent()
        content.title = notice.title
        content.subtitle = notice.subtitle
        content.body = notice.body
        content.threadIdentifier = notice.threadId
        var info = notice.info
        info[kindInfo] = notice.kind?.storedName
        content.userInfo = info
        content.sound = .default
        return content
    }

    /// The ids of the `requests` of one of `kinds`; one that says no kind is none of them.
    nonisolated internal static func ids(
        of requests: [UNNotificationRequest],
        ofKinds kinds: Set<NoticeKind>
    ) -> [String] {
        let names = Set(kinds.map(\.storedName))
        return requests.filter { request in
            (request.content.userInfo[kindInfo] as? String).map(names.contains) == true
        }
        .map(\.identifier)
    }

    // takes the notices `ids` down and leaves the categories `wanted`
    private static func remove(
        _ ids: [String],
        from center: UNUserNotificationCenter,
        keeping wanted: Set<UNNotificationCategory>
    ) {
        center.removePendingNotificationRequests(withIdentifiers: ids)
        center.removeDeliveredNotifications(withIdentifiers: ids)
        center.setNotificationCategories(wanted)
    }

    /// Posts `notice`, replacing one with its id.
    public func post(_ notice: Notice) {
        let content = Self.content(of: notice)
        if !notice.actions.isEmpty {
            let id = Self.category(of: notice.id)
            categories[id] = UNNotificationCategory(
                identifier: id,
                actions: notice.actions.map(Self.systemAction),
                intentIdentifiers: [],
                options: []
            )
            content.categoryIdentifier = id
        }
        let request = UNNotificationRequest(identifier: notice.id, content: content, trigger: nil)
        let wanted = Set(categories.values)
        enqueue { center in
            center.setNotificationCategories(wanted)
            // the categories are registered once they read back: a notice added before shows without its actions
            _ = await center.notificationCategories()
            try? await center.add(request)
        }
    }

    /// Takes down the notices `ids` that are up, and their categories.
    public func withdraw(_ ids: [String]) {
        guard !ids.isEmpty else {
            return
        }
        forget(ids)
        let wanted = Set(categories.values)
        enqueue { center in
            Self.remove(ids, from: center, keeping: wanted)
        }
    }

    /// Takes down every notice delivered of one of `kinds`, and their categories.
    public func withdraw(kinds: Set<NoticeKind>) {
        guard !kinds.isEmpty else {
            return
        }
        // a notice posted again after this call keeps its new category
        let known = categories
        enqueue { center in
            let ids = Self.ids(of: await center.deliveredNotifications().map(\.request), ofKinds: kinds)
            self.forget(ids, unlessChangedSince: known)
            Self.remove(ids, from: center, keeping: Set(self.categories.values))
        }
    }

    /// Waits until every post and withdrawal made so far has been handed to the system.
    public func delivered() async {
        await last?.value
    }

    /// Whether the user has answered the question about notifying.
    public func permission() async -> NoticePermission {
        switch await center.notificationSettings().authorizationStatus {
        case .notDetermined:
            .undecided

        case .denied:
            .denied

        default:
            .allowed
        }
    }

    /// Asks the user, once, whether the app may notify: alerts, sounds and the badge.
    public func requestPermission() async -> Bool {
        (try? await center.requestAuthorization(options: [.alert, .sound, .badge])) ?? false
    }

    // drops the categories of the notices `ids`
    private func forget(_ ids: [String]) {
        for id in ids {
            categories[Self.category(of: id)] = nil
        }
    }

    // drops the categories of the notices `ids` that are still the ones in `known`
    private func forget(_ ids: [String], unlessChangedSince known: [String: UNNotificationCategory]) {
        for id in ids {
            let key = Self.category(of: id)
            if let current = categories[key], current === known[key] {
                categories[key] = nil
            }
        }
    }

    // runs `step` once the post or withdrawal before it has
    private func enqueue(_ step: @escaping @MainActor (UNUserNotificationCenter) async -> Void) {
        let before = last
        let system = center
        last = Task {
            await before?.value
            await step(system)
        }
    }

    deinit {
        // nothing to release: the system keeps what was posted
    }
}
