import NgtermCore
import NgtermCoreFakes
import Testing

/// The fake core's Duplicate, and what it tells as a pane is shown and as its lead is taken (``FakeMobileCoreTests``
/// has the rest of its sessions).
@Suite("the fake core's sessions", .timeLimit(.minutes(1)))
internal struct FakeMobileCoreSessionTests {
    private static let machine = Machine(id: "m1", label: "box", host: "box.example", port: 22, user: "me", keyId: "k1")
    private static let size = PaneSize(cols: 80, rows: 24, cellWidth: 10, cellHeight: 20)

    @Test("a duplicate lands right after its source as a plain session of its own, delivered in the listing")
    internal func duplicateFollowsItsSource() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        var source = fakeSession(id: "s1", name: "api", parent: "s0", paneCount: 2)
        source.context = "Fixing the tests"
        core.sessions["m1"] = [fakeSession(id: "s0"), source, fakeSession(id: "s2")]
        let copy = try await core.duplicateSession(machineId: "m1", sessionId: "s1")
        #expect(copy.id == "fake-1")
        #expect(copy.name == "session 1")
        #expect(copy.parent == nil)
        #expect(copy.context == nil)
        #expect(copy.paneCount == 2)
        let ids = ["s0", "s1", "fake-1", "s2"]
        #expect(core.sessions["m1"]?.map(\.id) == ids)
        guard case .tree(_, let listed) = core.emitted.last else {
            Issue.record("no listing after the duplicate")
            return
        }
        #expect(listed.map(\.id) == ids)
        #expect(core.calls.last == FakeCall("duplicateSession", ["m1", "s1"]))
    }

    @Test("a Take control delivers what the origin tells of that pane's takeover, and of no other pane")
    internal func takeLeadDeliversItsEvents() throws {
        let core = FakeMobileCore()
        let pane = PaneRef(machineId: "m1", sessionId: "s1", paneId: "P1")
        let led = MobileEvent.paneLead(pane: pane, role: .leader, covered: false)
        core.takeLeadEvents = [pane: [led]]
        try core.takeLead(machineId: "m1", sessionId: "s1", paneId: "P2")
        #expect(core.emitted.isEmpty)
        try core.takeLead(machineId: "m1", sessionId: "s1", paneId: "P1")
        #expect(core.emitted == [led])
    }

    @Test("a show of a pane not shown yet delivers what the origin tells of it; a show of a shown one nothing more")
    internal func showDeliversItsEvents() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        core.sessions["m1"] = [fakeSession(id: "s1")]
        _ = try await core.listSessions(machineId: "m1")
        let pane = PaneRef(machineId: "m1", sessionId: "s1", paneId: "P1")
        let follows = MobileEvent.paneLead(pane: pane, role: .follower, covered: false)
        core.showEvents = [pane: [follows]]
        _ = try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "P1", size: Self.size)
        _ = try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "P1", size: Self.size)
        #expect(core.emitted.filter { $0 == follows }.count == 1)
    }

    @Test("duplicating a session the machine doesn't have is a remote error")
    internal func duplicatingAMissingSession() async throws {
        let core = FakeMobileCore()
        try await core.connect(machine: Self.machine)
        await #expect(throws: MobileError.Remote(reason: "no session nope")) {
            _ = try await core.duplicateSession(machineId: "m1", sessionId: "nope")
        }
    }
}
