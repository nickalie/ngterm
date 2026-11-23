import UserNotifications

/// The notices the system shows as delivered, which ``SystemNotices`` searches by kind and takes withdrawn notices out
/// of: the system's list, or a test's, as a test app's own notices need not reach the system's list.
@preconcurrency
@MainActor
public protocol DeliveredNotices: AnyObject {
    /// The requests of the notices delivered.
    func requests() async -> [UNNotificationRequest]

    /// Takes the notices `ids` out of the list.
    func remove(_ ids: [String])
}
