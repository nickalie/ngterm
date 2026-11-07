import Foundation
import NgtermPlatform
import Testing

/// The session chat views and the settings with their file there but unreadable: they start from nothing, refuse to
/// write over it, and take what it holds once it can be read.
@MainActor
@Suite("stores over an unreadable file")
internal struct UnreadableStoresTests {
    /// Makes the file `name` in `directory` unreadable (`readable` false) or readable again.
    private static func set(_ name: String, in directory: TemporaryDirectory, readable: Bool) throws {
        let permissions = readable ? 0o644 : 0o000
        try FileManager.default.setAttributes(
            [.posixPermissions: permissions],
            ofItemAtPath: directory.file(name).path
        )
    }

    @Test("the session chat views are read again on asking once the file can be, and only then")
    internal func chatViewsReadAgain() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let writer = SessionChatViewsStore(directory: directory.url)
        try writer.set(machine: "m1", session: "s1", chat: false, default: true)
        try Self.set(SessionChatViewsStore.fileName, in: directory, readable: false)
        let store = SessionChatViewsStore(directory: directory.url)
        store.readIfUnread()
        #expect(store.views.isEmpty, "still unreadable")
        try Self.set(SessionChatViewsStore.fileName, in: directory, readable: true)
        store.readIfUnread()
        #expect(store.choice(machine: "m1", session: "s1")?.chat == false)
        // read, the store keeps what it has: a file changed behind it isn't read again
        try writer.set(machine: "m1", session: "s1", chat: true, default: true)
        store.readIfUnread()
        #expect(store.choice(machine: "m1", session: "s1")?.chat == false)
    }

    @Test("a session chat view change refuses to write over an unreadable file, and starts from it once readable")
    internal func chatViewsUnreadable() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        try SessionChatViewsStore(directory: directory.url)
            .set(machine: "m1", session: "s1", chat: false, default: true)
        let file = directory.file(SessionChatViewsStore.fileName)
        let before = try Data(contentsOf: file)
        try Self.set(SessionChatViewsStore.fileName, in: directory, readable: false)
        let store = SessionChatViewsStore(directory: directory.url)
        #expect(store.views.isEmpty)
        #expect(throws: StoreError.self) {
            try store.set(machine: "m1", session: "s2", chat: false, default: true)
        }
        #expect(store.views.isEmpty)
        try Self.set(SessionChatViewsStore.fileName, in: directory, readable: true)
        #expect(try Data(contentsOf: file) == before)
        try store.set(machine: "m1", session: "s2", chat: false, default: true)
        #expect(store.views.map(\.session) == ["s1", "s2"])
        #expect(SessionChatViewsStore(directory: directory.url).views.map(\.session) == ["s1", "s2"])
    }

    @Test("a settings change refuses to write over an unreadable file, and starts from it once readable")
    internal func settingsUnreadable() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        try SettingsStore(directory: directory.url).update { $0.theme = .dark }
        let file = directory.file(SettingsStore.fileName)
        let before = try Data(contentsOf: file)
        try Self.set(SettingsStore.fileName, in: directory, readable: false)
        let store = SettingsStore(directory: directory.url)
        #expect(store.settings == AppSettings())
        #expect(throws: StoreError.self) {
            try store.update { $0.keepScreenOn.toggle() }
        }
        #expect(store.settings == AppSettings())
        try Self.set(SettingsStore.fileName, in: directory, readable: true)
        #expect(try Data(contentsOf: file) == before)
        try store.update { $0.keepScreenOn.toggle() }
        #expect(store.settings.theme == .dark)
        #expect(store.settings.keepScreenOn != AppSettings().keepScreenOn)
        #expect(SettingsStore(directory: directory.url).settings == store.settings)
    }
}
