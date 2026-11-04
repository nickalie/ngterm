import Foundation
import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// The core's events that the stream delivers after the app has moved on: what was told about a machine before its
/// disconnect changes nothing after it.
@MainActor
@Suite("the connections' late events")
internal struct LateEventsTests {
    private let machine = Machine(
        id: "m1", label: "build-01", host: "build-01.lan", port: 22, user: "nick", keyId: "k1"
    )
    private let clock = TestClock()
    private let core: FakeMobileCore
    private let connections: Connections

    internal init() {
        core = FakeMobileCore(hosts: MemoryHostKeyPins())
        connections = Connections(core: core, clock: clock.read)
    }

    @Test("what the core told before a disconnect, delivered after it, brings nothing back")
    internal func afterDisconnect() async {
        await connections.connect(machine).value
        await Wait.settled(connections, core)
        connections.disconnect(machineId: "m1")
        // the connect's Connected, a question and a listing, still in the stream when the user tapped Disconnect
        core.emit(.machineState(machineId: "m1", state: .connected))
        core.emit(.hostKeyUnknown(machineId: "m1", fingerprint: "SHA256:new"))
        core.emit(.hostKeyChanged(machineId: "m1", pinned: "SHA256:old", offered: "SHA256:new"))
        core.emit(.signInRequested(machineId: "m1", message: "sign in", url: "https://login.example/a"))
        core.emit(.tree(machineId: "m1", sessions: [fakeSession(id: "s1")]))
        await Wait.settled(connections, core)
        #expect(connections.links["m1"] == nil)
        #expect(connections.trees["m1"] == nil)
        // connected again, its events count once more
        await connections.connect(machine).value
        core.emit(.machineState(machineId: "m1", state: .reconnecting(nextInMs: 4_000)))
        await Wait.settled(connections, core)
        #expect(connections.links["m1"] == .reconnecting(retryAt: clock.now.addingTimeInterval(4)))
    }
}
