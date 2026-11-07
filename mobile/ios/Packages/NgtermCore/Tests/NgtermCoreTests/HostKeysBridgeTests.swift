import Testing

@testable import NgtermCore

/// The platform's host key pins as the core reaches them: what it reads, and the pin it keeps once the user trusts a
/// key.
@Suite("the host keys bridge")
internal struct HostKeysBridgeTests {
    @Test("the core reads the platform's pins and its pins reach the platform's store")
    internal func pinsPassThrough() {
        let hosts = MemoryHostKeys(["m1": "SHA256:one"])
        let bridge = HostKeysBridge(hosts)
        #expect(bridge.pinned(machineId: "m1") == "SHA256:one")
        #expect(bridge.pinned(machineId: "m2") == nil)
        bridge.pin(machineId: "m2", fingerprint: "SHA256:two")
        bridge.pin(machineId: "m1", fingerprint: "SHA256:new")
        #expect(hosts.pins == ["m1": "SHA256:new", "m2": "SHA256:two"])
    }
}
