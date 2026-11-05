import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// The session view's model over the fake core, as mobile/android's `SessionViewModelTest` has it.
@MainActor
@Suite("the session view's model")
internal struct SessionModelTests {
    private let world = SessionWorld()

    private func model() -> SessionModel {
        SessionModel(graph: world.graph, key: SessionWorld.key)
    }

    @Test("a live session selects its layout's primary pane until another is picked, and moves round its panes")
    internal func selection() async throws {
        defer { world.remove() }
        try await world.attach()
        let model = model()
        guard case .live(_, nil) = model.state else {
            Issue.record("no layout yet, no pane selected: \(String(describing: model.state))")
            return
        }
        await world.stream(SessionWorld.layout(["P1", "P2", "P3"], axis: "vertical", shown: true))
        #expect(model.focusedPane == "P1")
        #expect(model.select("P2"))
        #expect(!model.select("P2"), "selecting the selected pane moves nothing")
        #expect(model.focusedPane == "P2")
        #expect(model.neighbor(forward: true) == "P3")
        #expect(model.neighbor(forward: false) == "P1")
        model.select("P3")
        #expect(model.neighbor(forward: true) == "P1", "round from the last")
    }

    @Test("the selection outlives the view: a new view of the session (another size class) finds the same pane")
    internal func selectionOutlivesTheView() async throws {
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        model().select("P2")
        #expect(model().focusedPane == "P2")
        // a layout without it falls back to the primary
        let alone = SessionWorld.layout(["P1"], axis: nil, shown: true)
        await world.emit(.sessionLayout(machineId: "m1", sessionId: "s1", layout: alone))
        #expect(model().focusedPane == "P1")
    }

    @Test("its panes as the view places them: shown or not, with their state and failures")
    internal func spots() async throws {
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        await world.show("P1")
        await world.emit(.paneLead(pane: SessionWorld.pane("P2"), role: .follower, covered: true))
        let model = model()
        guard case .live(let session, _) = model.state else {
            Issue.record("not live")
            return
        }
        let spots = model.spots(of: session)
        #expect(spots.map(\.id) == ["P1", "P2"])
        #expect(spots[0].shown != nil)
        #expect(spots[0].cover == nil)
        #expect(spots[1].cover == .attaching, "not shown yet")
    }

    @Test("a session ended on its machine is gone, by the name it was last shown with; one detached closes")
    internal func goneAndClosed() async throws {
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        let model = model()
        #expect(model.state != nil)
        await world.emit(.paneEnded(pane: SessionWorld.pane("P1"), end: .gone))
        #expect(model.state == .gone(name: "api", machine: "build-01"))

        world.graph.connections.detach(machineId: "m1", sessionId: "s1")
        world.core.sessions["m1"] = []
        _ = try await world.core.listSessions(machineId: "m1")
        await world.test.settled()
        #expect(model.state == .gone(name: "api", machine: "build-01"), "the listing has dropped it")
    }

    @Test("a session detached while its machine still lists it closes")
    internal func detachCloses() async throws {
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        let model = model()
        model.detach()
        #expect(model.state == nil)
        #expect(world.core.calls.contains(FakeCall("detachSession", ["m1", "s1"])))
    }

    @Test("End session ends it, and a failure is the notice until dismissed")
    internal func end() async throws {
        defer { world.remove() }
        try await world.attach(["s1"], of: ["s1", "s2"])
        await world.stream(SessionWorld.split("vertical"))
        let model = model()
        world.core.failNext("endSession", with: .Remote(reason: "the session is busy"))
        #expect(await !model.end())
        #expect(model.notice == "the session is busy")
        model.dismissNotice()
        #expect(model.notice == nil)
        #expect(await model.end())
        #expect(!model.isEnding)
        #expect(world.graph.connections.attached["m1"] == nil)
    }

    @Test("Take control asks the core for the pane's lead")
    internal func takeLead() async throws {
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        let model = model()
        guard case .live(let session, _) = model.state else {
            Issue.record("not live")
            return
        }
        model.takeLead(model.spots(of: session)[1])
        #expect(world.core.calls.contains(FakeCall("takeLead", ["m1", "s1", "P2"])))
    }

    @Test("the next and previous attached sessions go round every machine's, in the lists' order")
    internal func sessionMoves() async throws {
        defer { world.remove() }
        try await world.attach(["s1", "s2"], of: ["s1", "s2", "s3"])
        let model = model()
        #expect(model.neighborSession(forward: true) == SessionKey(machineId: "m1", sessionId: "s2"))
        #expect(model.neighborSession(forward: false) == SessionKey(machineId: "m1", sessionId: "s2"))
        world.graph.connections.detach(machineId: "m1", sessionId: "s2")
        #expect(model.neighborSession(forward: true) == nil)
    }
}
