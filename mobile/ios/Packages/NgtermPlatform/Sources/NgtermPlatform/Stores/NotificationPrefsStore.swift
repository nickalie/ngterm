import Foundation
import Observation

/// The user's notification choices, kept as one JSON file in Application Support (`notifications.json`), apart from
/// the settings as on Android.
///
/// It reads the file as ``SettingsStore`` reads its own: a file that no longer decodes reads as the defaults (every
/// kind on, the permission not asked) and is kept as `notifications.json.bad`, and one that can't be read is left alone
/// until a change can read it. A change is written whole before ``prefs`` shows it.
@preconcurrency
@MainActor
@Observable
public final class NotificationPrefsStore {
    /// The store's file name.
    public static let fileName = "notifications.json"

    /// The choices.
    public private(set) var prefs: NotificationPrefs
    @ObservationIgnored private let file: JSONFile<NotificationPrefs>
    // whether `prefs` is what the file holds: false while the file is there but can't be read
    @ObservationIgnored private var loaded: Bool

    /// The store kept in `directory`: the app's Application Support unless a test gives its own.
    public init(directory: URL = .applicationSupportDirectory) {
        file = JSONFile(directory: directory, name: Self.fileName)
        do {
            prefs = try file.read() ?? NotificationPrefs()
            loaded = true
        } catch {
            prefs = NotificationPrefs()
            loaded = false
        }
    }

    /// Turns `kind` on or off.
    ///
    /// - Throws: ``StoreError`` when the change can't be written; the choices stay as they were.
    public func set(_ kind: NoticeKind, isOn: Bool) throws(StoreError) {
        try change { prefs in
            if isOn {
                prefs.off.remove(kind)
            } else {
                prefs.off.insert(kind)
            }
        }
    }

    /// The permission question has had its answer: it isn't asked again (Settings turns notifications on later).
    ///
    /// - Throws: ``StoreError`` when the change can't be written; the choices stay as they were.
    public func markPermissionAsked() throws(StoreError) {
        try change { $0.isPermissionAsked = true }
    }

    private func change(_ edit: (inout NotificationPrefs) -> Void) throws(StoreError) {
        if !loaded {
            prefs = try file.read() ?? NotificationPrefs()
            loaded = true
        }
        var next = prefs
        edit(&next)
        try file.write(next)
        prefs = next
    }

    deinit {
        // nothing to release: the file is written as each change is made
    }
}
