import Foundation
import NgtermPlatform
import Testing

/// The keys' store: what the app shows of its keys round-trips through `keys.json`.
@MainActor
@Suite("the key store")
internal struct KeyStoreTests {
    private static let made = AppKey(
        id: "k1", authorizedKey: "ssh-ed25519 AAAAmade ngterm@iPhone", fingerprint: "SHA256:made", isImported: false
    )
    private static let brought = AppKey(
        id: "k2", authorizedKey: "ssh-ed25519 AAAAbrought nick@laptop", fingerprint: "SHA256:brought", isImported: true
    )

    @Test("no file is no keys")
    internal func startsEmpty() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        #expect(KeyStore(directory: directory.url).keys.isEmpty)
        #expect(try directory.names().isEmpty)
    }

    @Test("the keys round-trip through the file, oldest first, and a known id is replaced in place")
    internal func roundTrip() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let store = KeyStore(directory: directory.url)
        try store.add(Self.made)
        try store.add(Self.brought)
        #expect(KeyStore(directory: directory.url).keys == [Self.made, Self.brought])
        let renewed = AppKey(
            id: "k1", authorizedKey: "ssh-ed25519 AAAAnew", fingerprint: "SHA256:new", isImported: false
        )
        try store.add(renewed)
        #expect(KeyStore(directory: directory.url).keys == [renewed, Self.brought])
        #expect(store.key(id: "k2") == Self.brought)
        #expect(store.key(id: "absent") == nil)
        #expect(try directory.names() == [KeyStore.fileName])
    }

    @Test("a corrupt file reads as no keys and is kept aside")
    internal func corruptFile() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        try Data("not json".utf8).write(to: directory.file(KeyStore.fileName))
        #expect(KeyStore(directory: directory.url).keys.isEmpty)
        #expect(try directory.names() == [KeyStore.fileName + ".bad"])
    }
}
