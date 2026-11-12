import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// The shown panes over the fake core, as mobile/android's `SessionPanesTest` has them.
@MainActor
@Suite("the shown panes")
internal struct SessionPanesTests {
    private let world = SessionWorld()

    private var panes: SessionPanes {
        world.graph.panes
    }

    private func shows() -> Int {
        world.core.calls.count { $0.method == "showPane" }
    }

    @Test("a pane is shown once, its frames routed to its feed, and stays shown")
    internal func showOnce() async throws {
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        await world.show("P1")
        panes.show(SessionWorld.pane("P1"), size: SessionWorld.size)
        await world.test.settled()
        #expect(shows() == 1)
        #expect(world.core.shownPanes == [SessionWorld.pane("P1")])
        #expect(panes.failures.isEmpty)
    }

    @Test("a pane the last listing doesn't know (a split since) is shown after a fresh listing")
    internal func unlistedPane() async throws {
        defer { world.remove() }
        try await world.attach()
        world.core.panes["s1"] = ["P1", "P2"]
        await world.stream(SessionWorld.split("vertical"))
        // the listing before the split knew no P2
        world.core.panes["s1"] = ["P1"]
        _ = try await world.core.listSessions(machineId: "m1")
        world.core.panes["s1"] = ["P1", "P2"]
        await world.show("P2")
        #expect(world.core.shownPanes.contains(SessionWorld.pane("P2")))
    }

    @Test("a pane that can't be shown keeps the core's words until Retry, which lists the machine first")
    internal func failureAndRetry() async throws {
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        world.core.failNext("showPane", with: .Remote(reason: "the pane is busy"))
        let pane = SessionWorld.pane("P1")
        panes.show(pane, size: SessionWorld.size)
        await Wait.until { panes.failures[pane] != nil }
        #expect(panes.failures[pane] == "the pane is busy")
        #expect(shows() == 1)

        let lists = world.core.calls.count { $0.method == "listSessions" }
        panes.retry(pane, size: SessionWorld.size)
        await Wait.until { panes.shown[pane] != nil }
        #expect(panes.failures[pane] == nil)
        #expect(world.core.calls.count { $0.method == "listSessions" } == lists + 1)
    }

    @Test("a machine whose link comes back lets go of its panes' failures")
    internal func linkBack() async throws {
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        world.core.failNext("showPane", with: .Remote(reason: "the pane is busy"))
        let pane = SessionWorld.pane("P1")
        panes.show(pane, size: SessionWorld.size)
        await Wait.until { panes.failures[pane] != nil }
        await world.emit(.machineState(machineId: "m1", state: .reconnecting(nextInMs: 1_000)))
        #expect(panes.failures[pane] != nil)
        await world.emit(.machineState(machineId: "m1", state: .connected))
        #expect(panes.failures[pane] == nil)
    }

    @Test("a pane the layout drops is hidden")
    internal func layoutDrops() async throws {
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        await world.show("P1", "P2")
        let alone = SessionWorld.layout(["P1"], axis: nil, shown: true)
        await world.emit(.sessionLayout(machineId: "m1", sessionId: "s1", layout: alone))
        #expect(panes.shown[SessionWorld.pane("P2")] == nil)
        #expect(panes.shown[SessionWorld.pane("P1")] != nil)
        #expect(world.core.calls.contains(FakeCall("hidePane", ["m1", "s1", "P2"])))
    }

    @Test("a claimed session's first pane to report a follower takes the lead, once; others are left as they are")
    internal func claimFollower() async throws {
        defer { world.remove() }
        try await world.attach(["s1", "s2"], of: ["s1", "s2"])
        let other = PaneRef(machineId: "m1", sessionId: "s2", paneId: "P1")
        await world.emit(.paneLead(pane: other, role: .follower, covered: true))
        panes.claimLead(of: SessionWorld.key)
        await world.emit(
            .paneLead(pane: SessionWorld.pane("P1"), role: nil, covered: true),
            .paneLead(pane: SessionWorld.pane("P1"), role: .unowned, covered: true)
        )
        #expect(!world.core.calls.contains { $0.method == "takeLead" })
        await world.emit(
            .paneLead(pane: SessionWorld.pane("P1"), role: .follower, covered: true),
            .paneLead(pane: SessionWorld.pane("P2"), role: .follower, covered: true)
        )
        #expect(world.core.calls.filter { $0.method == "takeLead" } == [FakeCall("takeLead", ["m1", "s1", "P1"])])
        #expect(panes.claims.isEmpty)
    }

    // the session's stream connected with `P1` shown, and no lead reported yet
    private func connectedUnreported() async throws {
        try await world.attach()
        let layout = SessionWorld.layout(["P1"], axis: nil, shown: true)
        await world.emit(
            .sessionConnection(machineId: "m1", sessionId: "s1", state: .connected),
            .sessionLayout(machineId: "m1", sessionId: "s1", layout: layout)
        )
        await world.show("P1")
    }

    private func cover() -> PaneCover? {
        let model = SessionModel(graph: world.graph, key: SessionWorld.key)
        guard case .live(let session, _) = model.state else {
            return nil
        }
        return model.spots(of: session).first?.cover
    }

    @Test("a claimed pane shows attaching, not Take control, until its takeover's next lead report, whatever it says")
    internal func claimHolds() async throws {
        defer { world.remove() }
        try await connectedUnreported()
        panes.claimLead(of: SessionWorld.key)
        await world.emit(.paneLead(pane: SessionWorld.pane("P1"), role: .follower, covered: true))
        #expect(cover() == .attaching)
        #expect(panes.isTakingLead(SessionWorld.key))
        // another device kept the lead: the cover is the user's to tap again
        await world.emit(.paneLead(pane: SessionWorld.pane("P1"), role: .follower, covered: true))
        #expect(cover() == .follower)
        #expect(!panes.isTakingLead(SessionWorld.key))
    }

    @Test("a claimed pane whose takeover leads shows its terminal")
    internal func claimTakes() async throws {
        defer { world.remove() }
        try await connectedUnreported()
        panes.claimLead(of: SessionWorld.key)
        await world.emit(
            .paneLead(pane: SessionWorld.pane("P1"), role: .follower, covered: true),
            .paneLead(pane: SessionWorld.pane("P1"), role: .leader, covered: false)
        )
        #expect(cover() == nil)
        #expect(!panes.isTakingLead(SessionWorld.key))
    }

    @Test("a takeover the core refuses holds nothing: Take control shows")
    internal func claimRefused() async throws {
        defer { world.remove() }
        try await connectedUnreported()
        panes.claimLead(of: SessionWorld.key)
        world.core.failNext("takeLead", with: .Stopped)
        await world.emit(.paneLead(pane: SessionWorld.pane("P1"), role: .follower, covered: true))
        #expect(cover() == .follower)
    }

    @Test("a lost link drops a claim and its takeover: another device may lead once it is back")
    internal func claimLostLink() async throws {
        defer { world.remove() }
        try await connectedUnreported()
        panes.claimLead(of: SessionWorld.key)
        await world.emit(.machineState(machineId: "m1", state: .reconnecting(nextInMs: 1_000)))
        #expect(!panes.isTakingLead(SessionWorld.key))
        panes.claimLead(of: SessionWorld.key)
        #expect(panes.claims.isEmpty, "no claim while the link is down")
        await world.emit(
            .machineState(machineId: "m1", state: .connected),
            .paneLead(pane: SessionWorld.pane("P1"), role: .follower, covered: true)
        )
        #expect(!world.core.calls.contains { $0.method == "takeLead" })
        panes.claimLead(of: SessionWorld.key)
        await world.emit(.paneLead(pane: SessionWorld.pane("P1"), role: .follower, covered: true))
        await world.emit(.machineState(machineId: "m1", state: .reconnecting(nextInMs: 1_000)))
        #expect(!panes.isTakingLead(SessionWorld.key))
    }

    @Test("a claimed session whose pane leads already is left as it is")
    internal func claimLeader() async throws {
        defer { world.remove() }
        try await world.attach()
        panes.claimLead(of: SessionWorld.key)
        await world.emit(.paneLead(pane: SessionWorld.pane("P1"), role: .leader, covered: false))
        #expect(panes.claims.isEmpty)
        await world.emit(.paneLead(pane: SessionWorld.pane("P1"), role: .follower, covered: true))
        #expect(!world.core.calls.contains { $0.method == "takeLead" })
    }

    @Test("a detached session's panes, failures, selection and claim go")
    internal func detachDrops() async throws {
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        await world.show("P1")
        panes.select("P2", of: SessionWorld.key)
        panes.claimLead(of: SessionWorld.key)
        world.graph.connections.detach(machineId: "m1", sessionId: "s1")
        #expect(panes.shown.isEmpty)
        #expect(panes.focus.isEmpty)
        #expect(panes.claims.isEmpty)
    }

    @Test("a pane whose session was detached while it was on its way is hidden again")
    internal func detachedOnTheWay() async throws {
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        let gate = world.core.hold("showPane")
        let pane = SessionWorld.pane("P1")
        panes.show(pane, size: SessionWorld.size)
        await gate.reached()
        world.graph.connections.detach(machineId: "m1", sessionId: "s1")
        gate.open()
        await Wait.until { world.core.calls.contains(FakeCall("hidePane", ["m1", "s1", "P1"])) }
        #expect(panes.shown[pane] == nil)
    }
}
