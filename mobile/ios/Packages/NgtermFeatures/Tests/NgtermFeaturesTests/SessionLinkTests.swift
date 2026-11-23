import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// A session link tapped in the session view, on the fake core (mobile/android's `SessionViewModelTest`): the session
/// it names opens, attached first when it is not; a view that moved on meanwhile opens nothing, and a failed attach
/// says why.
@MainActor
@Suite("Session links")
internal struct SessionLinkTests {
    private static let refused = MobileError.Remote(reason: "build-01 lost the session")
    private static let linked = SessionKey(machineId: "m1", sessionId: "s2")

    private let world = SessionWorld()

    private func view() -> SessionModel {
        let model = SessionModel(graph: world.graph, key: SessionWorld.key)
        model.setShowing(true)
        return model
    }

    @Test("a link attaches the session it names and opens it; an attached one opens as it is")
    internal func linkOpens() async throws {
        defer { world.remove() }
        try await world.attach(["s1"], of: ["s1", "s2"])
        let model = view()
        var opened: [SessionKey] = []
        await model.openLinked("s2") { opened.append($0) }.value
        #expect(opened == [Self.linked])
        #expect(world.graph.connections.attached["m1"] == ["s1", "s2"])
        await model.openLinked("s1") { opened.append($0) }.value
        #expect(opened == [Self.linked, SessionWorld.key])
        #expect(world.graph.connections.attached["m1"] == ["s1", "s2"])
    }

    @Test("a link whose attach finishes after the view was left stays attached and opens nothing")
    internal func viewLeft() async throws {
        defer { world.remove() }
        try await world.attach(["s1"], of: ["s1", "s2"])
        let model = view()
        let gate = world.core.hold("attachSession")
        let opening = model.openLinked("s2") { _ in Issue.record("a left view opened a link") }
        await gate.reached()
        model.setShowing(false)
        gate.open()
        await opening.value
        #expect(world.graph.connections.attached["m1"] == ["s1", "s2"])
    }

    @Test("a link whose attach finishes after the view moved to another session opens nothing")
    internal func viewMoved() async throws {
        defer { world.remove() }
        try await world.attach(["s1", "s2"], of: ["s1", "s2", "s3"])
        let model = view()
        let gate = world.core.hold("attachSession")
        let opening = model.openLinked("s3") { _ in Issue.record("a link took the moved-to session's place") }
        await gate.reached()
        var moved: [SessionKey] = []
        #expect(model.moveSession(forward: true) { moved.append($0) })
        gate.open()
        await opening.value
        #expect(moved == [Self.linked])
        #expect(world.graph.connections.attached["m1"] == ["s1", "s2", "s3"])
    }

    @Test("a link whose attach fails says why in the banner and opens nothing")
    internal func attachFails() async throws {
        defer { world.remove() }
        try await world.attach(["s1"], of: ["s1", "s2"])
        let model = view()
        world.core.failNext("attachSession", with: Self.refused)
        await model.openLinked("s2") { _ in Issue.record("a failed attach opened") }.value
        #expect(model.notice == Self.refused.reason)
        #expect(world.graph.connections.attached["m1"] == ["s1"])
    }
}
