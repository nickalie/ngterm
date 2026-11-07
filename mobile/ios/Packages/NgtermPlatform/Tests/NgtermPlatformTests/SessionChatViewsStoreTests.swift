import Foundation
import NgtermPlatform
import Testing

/// The sessions' own Agent chat view (mobile/android's `SessionChatViewsTest`): a choice round-trips through
/// `session-chat-views.json`, one equal to Settings' is dropped, and only the newest are kept.
@MainActor
@Suite("the session chat views store")
internal struct SessionChatViewsStoreTests {
    @Test("a session's own choice round-trips, and one set back to Settings' is dropped")
    internal func roundTrip() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let store = SessionChatViewsStore(directory: directory.url)
        #expect(store.choice(machine: "m1", session: "s1") == nil)
        try store.set(machine: "m1", session: "s1", chat: false, default: true)
        try store.set(machine: "m1", session: "s2", chat: true, default: false)
        #expect(store.choice(machine: "m1", session: "s1")?.chat == false)
        let read = SessionChatViewsStore(directory: directory.url)
        #expect(read.choice(machine: "m1", session: "s1")?.chat == false)
        #expect(read.choice(machine: "m1", session: "s2")?.chat == true)
        // the session set back to Settings' follows Settings again
        try store.set(machine: "m1", session: "s1", chat: true, default: true)
        #expect(store.choice(machine: "m1", session: "s1") == nil)
        #expect(SessionChatViewsStore(directory: directory.url).views.map(\.session) == ["s2"])
    }

    @Test("only the newest sessions are kept, a set one moving to the newest")
    internal func newestKept() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let store = SessionChatViewsStore(directory: directory.url)
        for index in 0...SessionChatViewsStore.maxSessions {
            try store.set(machine: "m1", session: "s\(index)", chat: false, default: true)
        }
        #expect(store.views.count == SessionChatViewsStore.maxSessions)
        #expect(store.choice(machine: "m1", session: "s0") == nil)
        try store.set(machine: "m1", session: "s1", chat: false, default: true)
        #expect(store.views.last?.session == "s1")
    }

    @Test("a corrupt file reads as no choices and is kept aside")
    internal func corruptFile() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        try Data("not json".utf8).write(to: directory.file(SessionChatViewsStore.fileName))
        #expect(SessionChatViewsStore(directory: directory.url).views.isEmpty)
        #expect(try directory.names() == [SessionChatViewsStore.fileName + ".bad"])
    }

    @Test("a change the store can't write throws and keeps the choices as they were")
    internal func unwritable() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let blocked = directory.file("blocked")
        try Data().write(to: blocked)
        let store = SessionChatViewsStore(directory: blocked)
        #expect(throws: StoreError.self) {
            try store.set(machine: "m1", session: "s1", chat: false, default: true)
        }
        #expect(store.views.isEmpty)
    }
}
