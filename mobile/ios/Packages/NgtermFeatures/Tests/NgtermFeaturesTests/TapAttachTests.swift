import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// A notification tap's session attached once its machine connects and lists it (``Connections/attachWhenListed``),
/// as after the app was started afresh for the tap: nothing attached yet, the machine still connecting.
@MainActor
@Suite("a tapped session's attach")
internal struct TapAttachTests {
    private let machine = Machine(
        id: "m1", label: "build-01", host: "build-01.lan", port: 22, user: "nick", keyId: "k1"
    )
    private let pins = MemoryHostKeyPins()
    private let core: FakeMobileCore
    private let connections: Connections
    private let showing = TapShowing()

    internal init() {
        core = FakeMobileCore(hosts: pins)
        connections = Connections(core: core)
        core.sessions["m1"] = [fakeSession(id: "s1")]
    }

    private func attaching(_ sessionId: String = "s1") -> Task<Bool, Never> {
        Task { [connections, showing] in
            await connections.attachWhenListed(machineId: "m1", sessionId: sessionId) {
                showing.isShown
            }
        }
    }

    @Test("it waits for the connect and the listing, then attaches")
    internal func waitsThenAttaches() async {
        let gate = core.hold("connect")
        let connecting = connections.connect(machine)
        await gate.reached()
        let attach = attaching()
        await Wait.settled(connections, core)
        #expect(connections.attached["m1"] == nil, "nothing is attached while the machine connects")
        gate.open()
        await connecting.value
        #expect(await attach.value)
        #expect(connections.attached["m1"] == ["s1"])
    }

    @Test("one attached already is attached at once")
    internal func attachedAlready() async throws {
        await connections.connect(machine).value
        await Wait.settled(connections, core)
        try await connections.attach(machineId: "m1", sessionId: "s1")
        #expect(await attaching().value)
        #expect(core.calls.count { $0.method == "attachSession" } == 1)
    }

    @Test("a session the listing no longer has attaches nothing")
    internal func notListed() async {
        let connecting = connections.connect(machine)
        let attach = attaching("s9")
        await connecting.value
        #expect(await attach.value == false)
        #expect(connections.attached["m1"] == nil)
    }

    @Test("a connect that needs the user attaches nothing")
    internal func needsTheUser() async {
        core.hostKeys["build-01.lan"] = "SHA256:new"
        _ = connections.connect(machine)
        #expect(await attaching().value == false)
        #expect(connections.links["m1"] == .hostKeyUnknown(fingerprint: "SHA256:new"))
        #expect(connections.attached["m1"] == nil)
        connections.disconnect(machineId: "m1")
    }

    @Test("the user going elsewhere, or a cancel, ends the wait with nothing attached")
    internal func goneElsewhere() async {
        let gate = core.hold("connect")
        let connecting = connections.connect(machine)
        await gate.reached()
        let left = attaching()
        let cancelled = attaching()
        await Wait.settled(connections, core)
        cancelled.cancel()
        #expect(await cancelled.value == false)
        showing.isShown = false
        #expect(await left.value == false)
        gate.open()
        await connecting.value
        await Wait.settled(connections, core)
        #expect(connections.attached["m1"] == nil)
    }
}
