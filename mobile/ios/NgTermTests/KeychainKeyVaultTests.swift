import Foundation
import NgtermCore
import NgtermPlatform
import Security
import Testing

/// The Keychain key vault, on the app's own Keychain (hosted: an unhosted test bundle has none), under a service of
/// each test's own.
@Suite("the Keychain key vault")
internal struct KeychainKeyVaultTests {
    private static let key = Data("-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n".utf8)

    private let probe = KeychainProbe()

    private var vault: KeychainKeyVault {
        KeychainKeyVault(service: probe.service)
    }

    @Test("a stored key loads as it was, and the bytes handed to the vault are wiped")
    internal func storeAndLoad() throws {
        defer { probe.removeAll() }
        var handed = Self.key
        try vault.store(keyId: "k1", privateKey: &handed)
        #expect(handed.isEmpty)
        #expect(try vault.load(keyId: "k1") == Self.key)
        #expect(try vault.has(keyId: "k1"))
    }

    @Test("a key the vault doesn't hold is missing, by its id")
    internal func missingKey() throws {
        defer { probe.removeAll() }
        #expect(throws: VaultError.Missing(keyId: "absent")) {
            try vault.load(keyId: "absent")
        }
        #expect(try !vault.has(keyId: "absent"))
    }

    @Test("storing a key again replaces it")
    internal func replace() throws {
        defer { probe.removeAll() }
        var first = Data("first".utf8)
        var second = Data("second".utf8)
        try vault.store(keyId: "k1", privateKey: &first)
        try vault.store(keyId: "k1", privateKey: &second)
        #expect(try vault.load(keyId: "k1") == Data("second".utf8))
        #expect(probe.count() == 1)
    }

    @Test("a deleted key is missing, and deleting it again does nothing")
    internal func delete() throws {
        defer { probe.removeAll() }
        var handed = Self.key
        try vault.store(keyId: "k1", privateKey: &handed)
        try vault.delete(keyId: "k1")
        #expect(throws: VaultError.Missing(keyId: "k1")) {
            try vault.load(keyId: "k1")
        }
        try vault.delete(keyId: "k1")
        #expect(probe.count() == 0)
    }

    @Test("a key is readable from the first unlock on, on this device only, and never synchronized")
    internal func itemAttributes() throws {
        defer { probe.removeAll() }
        var handed = Self.key
        try vault.store(keyId: "k1", privateKey: &handed)
        let attributes = probe.attributes(account: "k1")
        let accessible = attributes[kSecAttrAccessible as String] as? String
        #expect(accessible == kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String)
        #expect((attributes[kSecAttrSynchronizable as String] as? Bool) != true)
        #expect(attributes[kSecAttrService as String] as? String == probe.service)
    }

    @Test("vaults on two services keep their keys apart: one neither loads, has nor deletes the other's")
    internal func servicesApart() throws {
        let otherProbe = KeychainProbe()
        defer {
            probe.removeAll()
            otherProbe.removeAll()
        }
        let other = KeychainKeyVault(service: otherProbe.service)
        var handed = Self.key
        try vault.store(keyId: "k1", privateKey: &handed)
        #expect(throws: VaultError.Missing(keyId: "k1")) {
            try other.load(keyId: "k1")
        }
        #expect(try !other.has(keyId: "k1"))
        try other.delete(keyId: "k1")
        #expect(try vault.load(keyId: "k1") == Self.key)
        #expect(probe.count() == 1)
        #expect(otherProbe.count() == 0)
        #expect(KeychainKeyVault().service == KeychainKeyVault.appService)
    }
}
