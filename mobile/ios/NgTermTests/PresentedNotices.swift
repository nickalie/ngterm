import NgtermPlatform
import UserNotifications

/// Stands in as the notification center's delegate, to see the notifications the system presents to the app in the
/// foreground as they reach the user; ``restore()`` puts the hosted app's own delegate back.
///
/// A listed presentation keeps the notification in the system's delivered list, as one that reached the user out of
/// the app does.
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

    /// The system did not present the notice before the wait's deadline.
    internal struct TimedOut: Error {}

    private static let limitSeconds = 10
    private static let presentationLimit = Duration.seconds(limitSeconds)

    private let previous: (any UNUserNotificationCenterDelegate)?
    private let options: UNNotificationPresentationOptions
    private var shown: [String: Shown] = [:]
    private var waiting: [String: AsyncStream<Shown>.Continuation] = [:]

    /// Stands in from now on.
    override internal convenience init() {
        self.init(listed: false)
    }

    /// Stands in from now on; with `listed`, each notification presented stays in the delivered list.
    internal init(listed: Bool) {
        previous = UNUserNotificationCenter.current().delegate
        options = listed ? [.list] : []
        super.init()
        UNUserNotificationCenter.current().delegate = self
    }

    /// Puts the app's own delegate back.
    internal func restore() {
        UNUserNotificationCenter.current().delegate = previous
    }

    /// The notification `id` once the system presents it, bounded by the presentation limit.
    internal func presented(_ id: String) async throws -> Shown {
        try await presented(id, timeout: Self.presentationLimit)
    }

    /// The notification `id` once the system presents it, or an error on cancellation or after `timeout`; one presented
    /// before the wait returns at once.
    internal func presented(_ id: String, timeout: Duration) async throws -> Shown {
        try Task.checkCancellation()
        if let seen = shown[id] {
            return seen
        }
        let (presentations, continuation) = AsyncStream<Shown>.makeStream()
        waiting[id] = continuation
        let deadline = Task {
            try? await Task.sleep(for: timeout)
            continuation.finish()
        }
        defer {
            deadline.cancel()
            waiting[id] = nil
        }
        var iterator = presentations.makeAsyncIterator()
        let seen = await iterator.next()
        try Task.checkCancellation()
        guard let seen else {
            throw TimedOut()
        }
        return seen
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
        return await arrived(notification.request.identifier, seen)
    }

    private func arrived(_ id: String, _ seen: Shown) -> UNNotificationPresentationOptions {
        guard let waiter = waiting.removeValue(forKey: id) else {
            shown[id] = seen
            return options
        }
        waiter.yield(seen)
        waiter.finish()
        return options
    }

    deinit {
        // nothing to release: restore() hands the center back
    }
}
