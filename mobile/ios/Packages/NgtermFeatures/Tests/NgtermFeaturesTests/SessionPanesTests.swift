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

    @Test("a detached session's panes, failures and selection go")
    internal func detachDrops() async throws {
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        await world.show("P1")
        panes.select("P2", of: SessionWorld.key)
        world.graph.connections.detach(machineId: "m1", sessionId: "s1")
        #expect(panes.shown.isEmpty)
        #expect(panes.focus.isEmpty)
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
