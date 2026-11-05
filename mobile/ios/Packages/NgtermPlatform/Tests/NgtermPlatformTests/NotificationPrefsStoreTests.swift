import Foundation
import NgtermPlatform
import Testing

/// The notification choices' store: every kind on and the permission not asked until changed, kept across launches,
/// tolerant of a kind this build doesn't know and of a corrupt file.
@MainActor
@Suite("the notification choices' store")
internal struct NotificationPrefsStoreTests {
    @Test("no file is every kind on, the permission not asked")
    internal func defaults() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let prefs = NotificationPrefsStore(directory: directory.url).prefs
        #expect(prefs.enabled == Set(NoticeKind.allCases))
        #expect(!prefs.isPermissionAsked)
        #expect(try directory.names().isEmpty)
    }

    @Test("a kind turned off and the permission asked round-trip through the file")
    internal func roundTrip() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let store = NotificationPrefsStore(directory: directory.url)
        try store.set(.done, isOn: false)
        try store.set(.notices, isOn: false)
        try store.set(.notices, isOn: true)
        try store.markPermissionAsked()
        let read = NotificationPrefsStore(directory: directory.url).prefs
        #expect(read == NotificationPrefs(off: [.done], isPermissionAsked: true))
        #expect(read.enabled == [.errors, .notices, .waiting])
    }

    @Test("a kind this build doesn't know is dropped, and a corrupt file reads as the defaults and is kept aside")
    internal func unknownAndCorrupt() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let file = directory.file(NotificationPrefsStore.fileName)
        try Data(#"{"off": ["waiting", "someday"], "permissionAsked": true}"#.utf8).write(to: file)
        #expect(NotificationPrefsStore(directory: directory.url).prefs.off == [.waiting])
        try Data("not json".utf8).write(to: file)
        #expect(NotificationPrefsStore(directory: directory.url).prefs == NotificationPrefs())
        #expect(try directory.names() == [NotificationPrefsStore.fileName + ".bad"])
    }
}
