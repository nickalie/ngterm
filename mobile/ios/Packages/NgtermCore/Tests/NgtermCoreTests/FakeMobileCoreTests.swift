import NgtermCore
import NgtermCoreFakes
import Testing

/// The fake core's sessions, panes, calls and scripted events (mobile/android's FakeMobileCoreTest, its connects
/// in ``FakeMobileCoreConnectTests``).
@Suite("the fake core", .timeLimit(.minutes(1)))
internal struct FakeMobileCoreTests {
    private static let machine = Machine(id: "m1", label: "box", host: "box.example", port: 22, user: "me", keyId: "k1")
    private static let size = PaneSize(cols: 80, rows: 24, cellWidth: 10, cellHeight: 20)

    private static func streamOfADroppedFake(emitting events: [MobileEvent]) -> AsyncStream<MobileEvent> {
        let core = FakeMobileCore()
        for event in events {
            core.emit(event)
        }
        return core.events
    }

    @Test("sessions are created, listed and ended on a connected machine, each listing delivered as a tree")
    internal func createListAndEndSessions() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        let session = try await core.createSession(machineId: "m1", name: "work")
        #expect(session.name == "work")
        #expect(try await core.listSessions(machineId: "m1") == [session])
        try await core.endSession(machineId: "m1", sessionId: session.id)
        #expect(try await core.listSessions(machineId: "m1").isEmpty)
        #expect(
            core.emitted == [
                .machineState(machineId: "m1", state: .connected),
                .tree(machineId: "m1", sessions: [session]),
                .tree(machineId: "m1", sessions: [session]),
                .tree(machineId: "m1", sessions: []),
                .tree(machineId: "m1", sessions: []),
            ]
        )
        #expect(
            core.calls.map(\.method) == ["connect", "createSession", "listSessions", "endSession", "listSessions"]
        )
    }

    @Test("the calls are recorded with their arguments")
    internal func callsAreRecordedWithTheirArguments() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        _ = try await core.createSession(machineId: "m1", name: nil)
        let reply = AskReply(button: "yes", text: nil, selected: nil)
        try core.answerAsk(machineId: "m1", sessionId: "s1", askId: "A1", reply: reply)
        #expect(core.calls[1] == FakeCall("createSession", ["m1", String?.none]))
        #expect(core.calls[2] == FakeCall("answerAsk", ["m1", "s1", "A1", reply]))
        #expect(core.calls[2] != FakeCall("answerAsk", ["m1", "s1", "A2", reply]))
    }

    @Test("emitted events reach the stream in order, and the stream ends once the fake is dropped")
    internal func emittedEventsReachTheStream() async {
        let pane = PaneRef(machineId: "m1", sessionId: "s1", paneId: "P1")
        let scripted: [MobileEvent] = [.paneAttached(pane: pane), .paneDirty(pane: pane), .bell(pane: pane)]
        let events = Self.streamOfADroppedFake(emitting: scripted)
        var received: [MobileEvent] = []
        for await event in events {
            received.append(event)
        }
        #expect(received == scripted)
    }

    @Test("a blank name is none")
    internal func blankNameIsNone() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        #expect(try await core.createSession(machineId: "m1", name: " ").name == "session 1")
    }

    @Test("ending a session not listed needs no listing and delivers no tree")
    internal func endingAnUnlistedSession() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        core.sessions["m1"] = [fakeSession(id: "s1")]
        try await core.endSession(machineId: "m1", sessionId: "s1")
        #expect(core.sessions["m1"]?.isEmpty == true)
        #expect(core.emitted == [.machineState(machineId: "m1", state: .connected)])
    }

    @Test("ending a session the machine doesn't have is a remote error")
    internal func endingAMissingSession() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        await #expect(throws: MobileError.Remote(reason: "no session nope")) {
            try await core.endSession(machineId: "m1", sessionId: "nope")
        }
    }

    @Test("an attach takes only a session of the machine's last listing")
    internal func attachTakesOnlyAListedSession() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        core.sessions["m1"] = [fakeSession(id: "s1")]
        await #expect(throws: MobileError.NotListed(sessionId: "s1")) {
            try await core.attachSession(machineId: "m1", sessionId: "s1")
        }
        _ = try await core.listSessions(machineId: "m1")
        try await core.attachSession(machineId: "m1", sessionId: "s1")
        await #expect(throws: MobileError.NotListed(sessionId: "nope")) {
            try await core.attachSession(machineId: "m1", sessionId: "nope")
        }
        try core.disconnect(machineId: "m1")
        try await core.connect(machine: Self.machine)
        await #expect(throws: MobileError.NotListed(sessionId: "s1")) {
            try await core.attachSession(machineId: "m1", sessionId: "s1")
        }
    }

    @Test("a machine not connected is refused")
    internal func machineNotConnectedIsRefused() async {
        let core = FakeMobileCore()
        await #expect(throws: MobileError.NotConnected(machineId: "m1")) {
            try await core.listSessions(machineId: "m1")
        }
    }

    @Test("a queued failure is thrown once")
    internal func queuedFailureIsThrownOnce() async throws {
        let core = FakeMobileCore()
        core.failNext("connect", with: .Unreachable(reason: "no route"))
        await #expect(throws: MobileError.Unreachable(reason: "no route")) {
            try await core.connect(machine: Self.machine)
        }
        try await core.connect(machine: Self.machine)
        #expect(core.connected == ["m1"])
    }

    @Test("a held call is recorded at once and goes on once its gate opens")
    internal func heldCallWaitsForItsGate() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        let gate = core.hold("createSession")
        let creating = Task {
            try await core.createSession(machineId: "m1", name: "held")
        }
        await gate.reached()
        #expect(core.calls.last?.method == "createSession")
        #expect(core.sessions["m1"] == nil)
        gate.open()
        #expect(try await creating.value.name == "held")
        // the gate held one call only
        #expect(try await core.createSession(machineId: "m1", name: "next").name == "next")
    }

    @Test("a held call whose task is cancelled throws at once")
    internal func cancelledHeldCallThrows() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        let gate = core.hold("listSessions")
        let listing = Task {
            try await core.listSessions(machineId: "m1")
        }
        await gate.reached()
        listing.cancel()
        await #expect(throws: CancellationError.self) {
            try await listing.value
        }
        gate.open()
    }

    @Test("a call whose task was cancelled before it is not recorded, and leaves its failure and gate to the next")
    internal func callCancelledBeforeItIsNotMade() async throws {
        let core = FakeMobileCore()
        core.failNext("connect", with: .Unreachable(reason: "no route"))
        let gate = core.hold("connect")
        let cancelled = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            try await core.connect(machine: Self.machine)
        }
        await #expect(throws: CancellationError.self) {
            try await cancelled.value
        }
        #expect(core.calls.isEmpty)
        let connecting = Task {
            try await core.connect(machine: Self.machine)
        }
        await gate.reached()
        gate.open()
        await #expect(throws: MobileError.Unreachable(reason: "no route")) {
            try await connecting.value
        }
        #expect(core.calls == [FakeCall("connect", [Self.machine])])
    }

    @Test("a cancelled call is made before what its caller does next")
    @MainActor
    internal func cancelledCallComesBeforeTheNextOne() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        let gate = core.hold("showPane")
        // runs on the main actor up to its first suspension: the wait at the gate
        let showing = Task.immediate { @MainActor in
            try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "p1", size: Self.size)
        }
        showing.cancel()
        try core.hidePane(machineId: "m1", sessionId: "s1", paneId: "p1")
        await #expect(throws: CancellationError.self) {
            try await showing.value
        }
        #expect(core.calls.map(\.method) == ["connect", "showPane", "hidePane"])
        gate.open()
    }

    @Test("a shown pane is handed back as it is until hidden, and a detach or a disconnect hides it")
    internal func shownPanesAreKeptUntilHidden() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        core.sessions["m1"] = [fakeSession(id: "s1")]
        _ = try await core.listSessions(machineId: "m1")
        let first = try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "P1", size: Self.size)
        #expect(try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "P1", size: Self.size) === first)
        try core.hidePane(machineId: "m1", sessionId: "s1", paneId: "P1")
        #expect(try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "P1", size: Self.size) !== first)
        _ = try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "P2", size: Self.size)
        #expect(
            core.shownPanes == [
                PaneRef(machineId: "m1", sessionId: "s1", paneId: "P1"),
                PaneRef(machineId: "m1", sessionId: "s1", paneId: "P2"),
            ]
        )
        try core.detachSession(machineId: "m1", sessionId: "s1")
        #expect(core.shownPanes.isEmpty)
        _ = try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "P1", size: Self.size)
        try core.disconnect(machineId: "m1")
        #expect(core.shownPanes.isEmpty)
    }

    @Test("a pane added since the last listing is unknown until the machine is listed again")
    internal func paneAddedSinceTheListing() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        core.sessions["m1"] = [fakeSession(id: "s1")]
        core.panes["s1"] = ["P1"]
        _ = try await core.listSessions(machineId: "m1")
        _ = try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "P1", size: Self.size)
        core.panes["s1"] = ["P1", "P2"]
        await #expect(throws: MobileError.Remote(reason: FakeMobileCore.unknownPane)) {
            try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "P2", size: Self.size)
        }
        _ = try await core.listSessions(machineId: "m1")
        _ = try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "P2", size: Self.size)
        #expect(core.shownPanes.contains(PaneRef(machineId: "m1", sessionId: "s1", paneId: "P2")))
    }

    @Test("a shown pane is the factory's, a fake pane by default")
    internal func shownPaneComesFromTheFactory() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        core.sessions["m1"] = [fakeSession(id: "s1")]
        _ = try await core.listSessions(machineId: "m1")
        let plain = try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "P1", size: Self.size)
        #expect(plain.id() == "P1")
        let made = FakePane(paneId: "P2")
        core.paneFactory = { _, _, _, _ in made }
        #expect(try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "P2", size: Self.size) === made)
    }
}
