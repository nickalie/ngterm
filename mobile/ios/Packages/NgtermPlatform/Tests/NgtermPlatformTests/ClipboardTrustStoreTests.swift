import Foundation
import NgtermPlatform
import Testing

/// The machines trusted with the clipboard: they round-trip through `clipboard-trust.json`, and a file that can't be
/// read trusts none.
@MainActor
@Suite("the clipboard trust store")
internal struct ClipboardTrustStoreTests {
    @Test("no file trusts no machine")
    internal func startsEmpty() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        #expect(ClipboardTrustStore(directory: directory.url).machines.isEmpty)
        #expect(try directory.names().isEmpty)
    }

    @Test("allow and revoke round-trip through the file, outside backups")
    internal func roundTrip() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let store = ClipboardTrustStore(directory: directory.url)
        try store.allow("m2")
        try store.allow("m1")
        try store.allow("m1")
        #expect(store.machines == ["m1", "m2"])
        #expect(ClipboardTrustStore(directory: directory.url).machines == ["m1", "m2"])
        try store.revoke("m2")
        try store.revoke("m9")
        #expect(ClipboardTrustStore(directory: directory.url).machines == ["m1"])
        let values = try directory.file(ClipboardTrustStore.fileName)
            .resourceValues(forKeys: [.isExcludedFromBackupKey])
        #expect(values.isExcludedFromBackup == true)
    }

    @Test("a corrupt file trusts no machine and is kept aside")
    internal func corruptFile() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        try Data("not json".utf8).write(to: directory.file(ClipboardTrustStore.fileName))
        #expect(ClipboardTrustStore(directory: directory.url).machines.isEmpty)
        #expect(try directory.names() == [ClipboardTrustStore.fileName + ".bad"])
    }

    @Test("a change the store can't write throws and trusts as before")
    internal func unwritable() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let blocked = directory.file("blocked")
        try Data().write(to: blocked)
        let store = ClipboardTrustStore(directory: blocked)
        #expect(throws: StoreError.self) {
            try store.allow("m1")
        }
        #expect(store.machines.isEmpty)
    }
}
