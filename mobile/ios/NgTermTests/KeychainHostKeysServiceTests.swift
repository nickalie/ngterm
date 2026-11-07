import NgtermPlatform
import Testing

/// The Keychain service the host key pins are kept under: the app's own, or a test's.
@Suite("the Keychain host keys' service")
internal struct KeychainHostKeysServiceTests {
    @Test("the pins are kept under the app's service unless one is given, and land under the one given")
    internal func service() throws {
        #expect(KeychainHostKeys().service == KeychainHostKeys.appService)
        let probe = KeychainProbe()
        defer { probe.removeAll() }
        let hosts = KeychainHostKeys(service: probe.service)
        #expect(hosts.service == probe.service)
        try hosts.set(machineId: "m1", fingerprint: "SHA256:one")
        #expect(probe.count() == 1)
    }
}
