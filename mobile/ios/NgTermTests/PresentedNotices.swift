import NgtermPlatform
import UserNotifications

/// Stands in as the notification center's delegate, to see the notifications the system presents to the app in the
/// foreground as they reach the user; ``restore()`` puts the hosted app's own delegate back.
@MainActor
internal final class PresentedNotices: NSObject, UNUserNotificationCenterDelegate {
    /// A presented notification, as the user sees it.
    internal struct Shown: Equatable, Sendable {
        /// Its title.
        internal var title: String
        /// The line under its title.
        internal var subtitle: String
        /// Its text.
        internal var body: String
        /// The thread it groups under.
        internal var threadId: String
        /// The category of its buttons; empty for none.
        internal var category: String
        /// What its tap carries back, as the app reads it.
        internal var info: [String: String]
    }

    private let previous: (any UNUserNotificationCenterDelegate)?
    private var shown: [String: Shown] = [:]
    private var waiting: [String: CheckedContinuation<Shown, Never>] = [:]

    /// Stands in from now on.
    override internal init() {
        previous = UNUserNotificationCenter.current().delegate
        super.init()
        UNUserNotificationCenter.current().delegate = self
    }

    /// Puts the app's own delegate back.
    internal func restore() {
        UNUserNotificationCenter.current().delegate = previous
    }

    /// The notification `id` once the system presents it.
    internal func presented(_ id: String) async -> Shown {
        if let seen = shown[id] {
            return seen
        }
        return await withCheckedContinuation { continuation in
            waiting[id] = continuation
        }
    }

    nonisolated internal func userNotificationCenter(
        _: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        let content = notification.request.content
        let seen = Shown(
            title: content.title,
            subtitle: content.subtitle,
            body: content.body,
            threadId: content.threadIdentifier,
            category: content.categoryIdentifier,
            info: SystemNotices.info(of: notification)
        )
        await arrived(notification.request.identifier, seen)
        return []
    }

    private func arrived(_ id: String, _ seen: Shown) {
        guard let waiter = waiting.removeValue(forKey: id) else {
            shown[id] = seen
            return
        }
        waiter.resume(returning: seen)
    }

    deinit {
        // nothing to release: restore() hands the center back
    }
}
