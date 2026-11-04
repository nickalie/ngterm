import Foundation
import NgtermPlatform
import Testing

/// The attached sessions' store: the set round-trips through `attached.json`, for a background refresh to read.
@MainActor
@Suite("the attached sessions' store")
internal struct AttachedStoreTests {
    @Test("no file is nothing attached")
    internal func startsEmpty() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        #expect(AttachedStore(directory: directory.url).attached.isEmpty)
        #expect(try directory.names().isEmpty)
    }

    @Test("the set round-trips through the file, without the machines that have none, outside backups")
    internal func roundTrip() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let store = AttachedStore(directory: directory.url)
        try store.save(["m1": ["s2", "s1"], "m2": [], "m3": ["s9"]])
        #expect(store.attached == ["m1": ["s1", "s2"], "m3": ["s9"]])
        #expect(AttachedStore(directory: directory.url).attached == ["m1": ["s1", "s2"], "m3": ["s9"]])
        try store.save([:])
        #expect(AttachedStore(directory: directory.url).attached.isEmpty)
        #expect(try directory.names() == [AttachedStore.fileName])
        let values = try directory.file(AttachedStore.fileName).resourceValues(forKeys: [.isExcludedFromBackupKey])
        #expect(values.isExcludedFromBackup == true)
    }

    @Test("a corrupt file reads as nothing attached and is kept aside")
    internal func corruptFile() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        try Data("not json".utf8).write(to: directory.file(AttachedStore.fileName))
        #expect(AttachedStore(directory: directory.url).attached.isEmpty)
        #expect(try directory.names() == [AttachedStore.fileName + ".bad"])
    }

    @Test("a set the store can't write throws and leaves the set as it was")
    internal func unwritable() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        // a file where the store's directory should be
        let blocked = directory.file("blocked")
        try Data().write(to: blocked)
        let store = AttachedStore(directory: blocked)
        #expect(throws: StoreError.self) {
            try store.save(["m1": ["s1"]])
        }
        #expect(store.attached.isEmpty)
    }
}
