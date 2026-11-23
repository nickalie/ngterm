import UserNotifications

/// The system's delivered list.
@MainActor
internal final class SystemDeliveredNotices: DeliveredNotices {
    private let center = UNUserNotificationCenter.current()

    /// The requests of the notices in the system's list.
    internal func requests() async -> [UNNotificationRequest] {
        await center.deliveredNotifications().map(\.request)
    }

    /// Takes the notices `ids` out of the system's list.
    internal func remove(_ ids: [String]) {
        center.removeDeliveredNotifications(withIdentifiers: ids)
    }

    deinit {
        // nothing to release: the system keeps its list
    }
}
