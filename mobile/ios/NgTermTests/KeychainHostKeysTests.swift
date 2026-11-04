import NgtermCore
import NgtermPlatform
import Security
import Testing

/// The host keys pinned in the Keychain, on the app's own Keychain (hosted), under a service of each test's own.
@Suite("the Keychain host keys")
internal struct KeychainHostKeysTests {
    private static let fingerprint = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU"

    private let probe = KeychainProbe()

    private var hosts: KeychainHostKeys {
        KeychainHostKeys(service: probe.service)
    }

    @Test("a pin is looked up by its machine, and other machines have none")
    internal func pinAndLookUp() throws {
        defer { probe.removeAll() }
        hosts.pin(machineId: "m1", fingerprint: Self.fingerprint)
        #expect(hosts.pinned(machineId: "m1") == Self.fingerprint)
        #expect(try hosts.fingerprint(machineId: "m1") == Self.fingerprint)
        #expect(hosts.pinned(machineId: "m2") == nil)
    }

    @Test("pinning again replaces the pin")
    internal func repin() throws {
        defer { probe.removeAll() }
        hosts.pin(machineId: "m1", fingerprint: "SHA256:old")
        try hosts.set(machineId: "m1", fingerprint: Self.fingerprint)
        #expect(hosts.pinned(machineId: "m1") == Self.fingerprint)
        #expect(probe.count() == 1)
    }

    @Test("a removed pin is gone, and removing it again does nothing")
    internal func remove() throws {
        defer { probe.removeAll() }
        hosts.pin(machineId: "m1", fingerprint: Self.fingerprint)
        hosts.pin(machineId: "m2", fingerprint: Self.fingerprint)
        try hosts.remove(machineId: "m1")
        try hosts.remove(machineId: "m1")
        #expect(hosts.pinned(machineId: "m1") == nil)
        #expect(hosts.pinned(machineId: "m2") == Self.fingerprint)
    }

    @Test("a pin is readable from the first unlock on, on this device only, and never synchronized")
    internal func itemAttributes() {
        defer { probe.removeAll() }
        hosts.pin(machineId: "m1", fingerprint: Self.fingerprint)
        let attributes = probe.attributes(account: "m1")
        let accessible = attributes[kSecAttrAccessible as String] as? String
        #expect(accessible == kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String)
        #expect((attributes[kSecAttrSynchronizable as String] as? Bool) != true)
    }

    @Test("the core reads the pins through its host-key store")
    internal func asTheCoresStore() {
        defer { probe.removeAll() }
        let store: any HostKeyStore = hosts
        store.pin(machineId: "m1", fingerprint: Self.fingerprint)
        #expect(store.pinned(machineId: "m1") == Self.fingerprint)
    }
}
