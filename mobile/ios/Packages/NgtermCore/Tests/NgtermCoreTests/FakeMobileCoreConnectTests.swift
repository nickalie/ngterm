import NgtermCore
import NgtermCoreFakes
import Testing

/// The fake core's connects: states, edits, host keys and sign-ins (mobile/android's FakeMobileCoreTest).
@Suite("the fake core's connects", .timeLimit(.minutes(1)))
internal struct FakeMobileCoreConnectTests {
    private static let machine = Machine(id: "m1", label: "box", host: "box.example", port: 22, user: "me", keyId: "k1")
    private static let connected = MobileEvent.machineState(machineId: "m1", state: .connected)

    private static func unreachable(_ reason: String) -> MobileEvent {
        .machineState(machineId: "m1", state: .unreachable(reason: reason))
    }

    @Test("a connect reports the state once, and a disconnect reports nothing")
    internal func connectReportsTheStateOnce() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        try await core.connect(machine: Self.machine)
        try core.disconnect(machineId: "m1")
        #expect(core.emitted == [Self.connected])
    }

    @Test("a failed connect reports the machine unreachable")
    internal func failedConnectReportsUnreachable() async {
        let core = FakeMobileCore()
        core.failNext("connect", with: .Unreachable(reason: "no route"))
        await #expect(throws: MobileError.Unreachable(reason: "no route")) {
            try await core.connect(machine: Self.machine)
        }
        #expect(core.emitted == [Self.unreachable("no route")])
    }

    @Test("an edited machine connects anew; one only renamed, or given another key, keeps its connection")
    internal func editedMachineConnectsAnew() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        var renamed = Self.machine
        renamed.label = "renamed"
        try await core.connect(machine: renamed)
        try core.useKey(machineId: "m1", keyId: "k2")
        var rekeyed = renamed
        rekeyed.keyId = "k2"
        try await core.connect(machine: rekeyed)
        #expect(core.emitted == [Self.connected])
        var edited = rekeyed
        edited.port = 2_222
        try await core.connect(machine: edited)
        #expect(core.emitted == [Self.connected, Self.connected])
    }

    @Test("an unknown host key waits for the answer, and trusting it pins it")
    internal func unknownHostKeyWaitsForTheAnswer() async throws {
        let pins = MemoryHostKeys()
        let core = FakeMobileCore(hosts: pins)
        core.hostKeys["box.example"] = "SHA256:new"
        var events = core.events.makeAsyncIterator()
        let connecting = Task {
            try await core.connect(machine: Self.machine)
        }
        #expect(await events.next() == .hostKeyUnknown(machineId: "m1", fingerprint: "SHA256:new"))
        #expect(core.waitingForHostKey == ["m1": "SHA256:new"])
        try core.answerHostKey(machineId: "m1", fingerprint: "SHA256:new", trust: true)
        try await connecting.value
        #expect(pins.pins == ["m1": "SHA256:new"])
        #expect(core.emitted.last == Self.connected)
        // pinned now: a later connect asks nothing
        try core.disconnect(machineId: "m1")
        try await core.connect(machine: Self.machine)
        #expect(core.emitted.count { if case .hostKeyUnknown = $0 { true } else { false } } == 1)
    }

    @Test(
        "a refused answer, or one about another key, fails the connect",
        arguments: [("SHA256:new", false), ("SHA256:other", true)]
    )
    internal func refusedOrMismatchedAnswerFails(fingerprint: String, trust: Bool) async throws {
        let pins = MemoryHostKeys()
        let core = FakeMobileCore(hosts: pins)
        core.hostKeys["box.example"] = "SHA256:new"
        var events = core.events.makeAsyncIterator()
        let connecting = Task {
            try await core.connect(machine: Self.machine)
        }
        _ = await events.next()
        try core.answerHostKey(machineId: "m1", fingerprint: fingerprint, trust: trust)
        await #expect(throws: MobileError.Unreachable(reason: FakeMobileCore.hostKeyRefused)) {
            try await connecting.value
        }
        #expect(core.emitted.last == Self.unreachable(FakeMobileCore.hostKeyRefused))
        #expect(pins.pins.isEmpty)
    }

    @Test("a disconnect cancels a connect waiting on its host key")
    internal func disconnectCancelsTheHostKeyWait() async throws {
        let core = FakeMobileCore(hosts: MemoryHostKeys())
        core.hostKeys["box.example"] = "SHA256:new"
        var events = core.events.makeAsyncIterator()
        let connecting = Task {
            try await core.connect(machine: Self.machine)
        }
        _ = await events.next()
        try core.disconnect(machineId: "m1")
        await #expect(throws: MobileError.Cancelled) {
            try await connecting.value
        }
        #expect(core.waitingForHostKey.isEmpty)
    }

    @Test("a changed host key reports both keys and fails")
    internal func changedHostKeyFails() async {
        let pins = MemoryHostKeys(["m1": "SHA256:old"])
        let core = FakeMobileCore(hosts: pins)
        core.hostKeys["box.example"] = "SHA256:new"
        await #expect(throws: MobileError.Unreachable(reason: FakeMobileCore.hostKeyChanged)) {
            try await core.connect(machine: Self.machine)
        }
        #expect(
            core.emitted == [
                .hostKeyChanged(machineId: "m1", pinned: "SHA256:old", offered: "SHA256:new"),
                Self.unreachable(FakeMobileCore.hostKeyChanged),
            ]
        )
        #expect(core.connected.isEmpty)
        #expect(pins.pins == ["m1": "SHA256:old"])
    }

    @Test("a sign-in waits for the user and connects once finished")
    internal func signInWaitsForTheUser() async throws {
        let core = FakeMobileCore()
        core.signIns["box.example"] = .tailscale
        var events = core.events.makeAsyncIterator()
        let connecting = Task {
            try await core.connect(machine: Self.machine)
        }
        #expect(
            await events.next()
                == .signInRequested(
                    machineId: "m1",
                    message: FakeSignIn.tailscale.message,
                    url: FakeSignIn.tailscale.url
                )
        )
        #expect(core.waitingForSignIn == ["m1"])
        core.finishSignIn(machineId: "m1")
        try await connecting.value
        #expect(core.emitted.last == Self.connected)
    }

    @Test("a sign-in not finished fails the connect, and a disconnect cancels one waiting")
    internal func unfinishedSignInFails() async throws {
        let core = FakeMobileCore()
        core.signIns["box.example"] = .tailscale
        var events = core.events.makeAsyncIterator()
        let failing = Task {
            try await core.connect(machine: Self.machine)
        }
        _ = await events.next()
        core.finishSignIn(machineId: "m1", signedIn: false)
        await #expect(throws: MobileError.Unreachable(reason: FakeMobileCore.signInUnfinished)) {
            try await failing.value
        }
        #expect(core.emitted.last == Self.unreachable(FakeMobileCore.signInUnfinished))
        _ = await events.next()
        let cancelled = Task {
            try await core.connect(machine: Self.machine)
        }
        _ = await events.next()
        try core.disconnect(machineId: "m1")
        await #expect(throws: MobileError.Cancelled) {
            try await cancelled.value
        }
        #expect(core.waitingForSignIn.isEmpty)
    }
}
