import Foundation
import NgtermPlatform

/// The small stores the app graph's parts keep their state in, beside the machines, the settings and the keys: the
/// attached set, the machines trusted with the clipboard, the notification choices and the status baseline.
@preconcurrency
@MainActor
public struct GraphStores {
    /// The sessions the app had attached, for a background refresh.
    public let attached: AttachedStore
    /// The machines whose programs read the clipboard unasked.
    public let clipboardTrust: ClipboardTrustStore
    /// The notification kinds turned off, and whether the app asked to notify.
    public let notificationPrefs: NotificationPrefsStore
    /// The last agent status seen of each attached session.
    public let statusBaseline: StatusBaselineStore

    /// Every store kept in `directory`: the app's Application Support unless a test gives its own.
    public init(directory: URL = .applicationSupportDirectory) {
        attached = AttachedStore(directory: directory)
        clipboardTrust = ClipboardTrustStore(directory: directory)
        notificationPrefs = NotificationPrefsStore(directory: directory)
        statusBaseline = StatusBaselineStore(directory: directory)
    }
}
