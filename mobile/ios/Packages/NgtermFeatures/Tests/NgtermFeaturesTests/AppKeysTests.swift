import Foundation
import NgtermCore
import NgtermFeatures
import NgtermPlatform
import SwiftUI
import Testing

/// The app's keys: made, imported with a passphrase and rotated, their public halves kept in the store and their
/// private halves in the vault, as mobile/android's stores test has them.
@MainActor
@Suite("the app's keys")
internal struct AppKeysTests {
    private let directory = FileManager.default.temporaryDirectory.appending(path: "AppKeysTests-\(UUID().uuidString)")
    private let vault = MemoryPrivateKeys()
    private let maker = FakeKeyMaker()

    private func keys(directory: URL? = nil) -> AppKeys {
        AppKeys(
            store: KeyStore(directory: directory ?? self.directory), vault: vault, maker: maker, comment: "ngterm@test"
        )
    }

    @Test("a made key is kept with its public half, and its private half in the vault")
    internal func generate() async throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let keys = keys()
        let made = try await keys.generate()
        #expect(!made.isImported)
        #expect(made.authorizedKey == FakeKeyMaker.publicKey)
        #expect(made.fingerprint == FakeKeyMaker.fingerprint)
        #expect(maker.commented == ["ngterm@test"])
        #expect(vault.privateKey(keyId: made.id) == Data("fake private key 1".utf8))
        #expect(keys.keys == [made])
        #expect(keys.key(id: made.id) == made)
        // the store keeps it for the next launch
        #expect(self.keys().keys == [made])
    }

    @Test("an import opens the key with its passphrase and zeroes what it read, whatever the outcome")
    internal func importWithPassphrase() async throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let keys = keys()
        var refused = Data(FakeKeyMaker.encryptedKey.utf8)
        await #expect(throws: KeyError.PassphraseRequired) {
            try await keys.importKey(privateKey: &refused, passphrase: "")
        }
        #expect(!refused.isEmpty && refused.allSatisfy { $0 == 0 })
        var wrong = Data(FakeKeyMaker.encryptedKey.utf8)
        await #expect(throws: KeyError.WrongPassphrase) {
            try await keys.importKey(privateKey: &wrong, passphrase: "wrong")
        }
        var notAKey = Data("not a key".utf8)
        await #expect(throws: KeyError.Invalid(reason: "unknown key format")) {
            try await keys.importKey(privateKey: &notAKey, passphrase: nil)
        }
        #expect(keys.keys.isEmpty)
        #expect(vault.keyIds.isEmpty)
        var bytes = Data(FakeKeyMaker.encryptedKey.utf8)
        let imported = try await keys.importKey(privateKey: &bytes, passphrase: FakeKeyMaker.passphrase)
        #expect(bytes.allSatisfy { $0 == 0 })
        #expect(imported.isImported)
        #expect(imported.authorizedKey.hasSuffix(" imported"))
        #expect(vault.privateKey(keyId: imported.id) == Data(FakeKeyMaker.encryptedKey.utf8))
        #expect(keys.keys == [imported])
    }

    @Test("a rotated key is a new one beside the others, and the machines keep theirs")
    internal func rotate() async throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let keys = keys()
        let machines = MachineStore(directory: directory)
        let first = try await keys.generate()
        try machines.save(
            StoredMachine(id: "m1", label: "", host: "build-01.lan", port: 22, user: "nick", keyId: first.id)
        )
        let rotated = try await keys.rotate()
        #expect(rotated.id != first.id)
        #expect(!rotated.isImported)
        #expect(keys.keys == [first, rotated])
        #expect(vault.keyIds == [first.id, rotated.id])
        #expect(machines.machines.map(\.keyId) == [first.id])
        try machines.useKey(rotated.id)
        #expect(machines.machines.map(\.keyId) == [rotated.id])
    }

    @Test("a key the store can't take leaves the vault too")
    internal func storeRefuses() async throws {
        // a file where the store's directory should be: nothing can be written under it
        let blocked = directory.appending(path: "blocked")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        try Data().write(to: blocked)
        let keys = keys(directory: blocked)
        await #expect(throws: StoreError.self) {
            try await keys.generate()
        }
        #expect(keys.keys.isEmpty)
        #expect(vault.keyIds.isEmpty)
    }

    @Test("the public key's command appends it to authorized_keys, quoting what the shell would read")
    internal func appendCommand() {
        let quoted = "echo 'ssh-ed25519 AAAA me@it'\\''s' >> ~/.ssh/authorized_keys"
        #expect(
            PublicKeyView<EmptyView>.appendCommand(for: "ssh-ed25519 AAAA me@it's")
                == "mkdir -p ~/.ssh && chmod 700 ~/.ssh && \(quoted)"
        )
    }
}
