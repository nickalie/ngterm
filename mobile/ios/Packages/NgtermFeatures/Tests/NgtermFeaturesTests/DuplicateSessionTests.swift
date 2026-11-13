import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// Duplicate session on the fake core, from a machine's list and from the session view's More, as mobile/android's
/// `DuplicateSessionFlowTest` has it: the copy opens in control, one create runs at a time, and a failure says why.
@MainActor
@Suite("Duplicate session")
internal struct DuplicateSessionTests {
    private static let refused = MobileError.Remote(reason: "build-01 could not copy api")
    private static let copy = SessionKey(machineId: "m1", sessionId: "fake-1")

    private let world = SessionWorld()
    private let windows = SessionWindows()

    private var graph: AppGraph {
        world.graph
    }

    private var duplicates: [[String]] {
        world.core.calls.filter { $0.method == "duplicateSession" }.map { $0.args.compactMap { $0 as? String } }
    }

    private func list() -> SessionsModel {
        let model = SessionsModel(graph: graph, machineId: "m1", windows: windows)
        model.setShowing(true)
        return model
    }

    private func view() -> SessionModel {
        let model = SessionModel(graph: graph, key: SessionWorld.key)
        model.setShowing(true)
        return model
    }

    private func line(_ sessionId: String, of model: SessionsModel) -> SessionLine? {
        guard let state = model.state, case .sessions(let attached, let others, _) = SessionsBody(state) else {
            return nil
        }
        return (attached + others).first { $0.id == sessionId }
    }

    @Test("a list's Duplicate says so on its row, holds every create, then opens the copy in control")
    internal func listDuplicateLands() async throws {
        defer { world.remove() }
        try await world.attach([], of: ["s1", "s2"])
        let model = list()
        let gate = world.core.hold("duplicateSession")
        var opened: [String] = []
        let duplicating = model.duplicate(sessionId: "s1") { opened.append($0) }
        await gate.reached()
        #expect(line("s1", of: model)?.isDuplicating == true)
        #expect(line("s2", of: model)?.isDuplicating == false)
        #expect(line("s2", of: model)?.canDuplicate == false)
        #expect(model.isCreateUnderWay)
        #expect(model.duplicate(sessionId: "s2") { _ in Issue.record("a second create began") } == nil)
        #expect(model.createAndOpen { _ in Issue.record("a New session began") } == nil)
        gate.open()
        await duplicating?.value
        #expect(opened == ["fake-1"])
        #expect(graph.panes.claims == [Self.copy])
        #expect(graph.connections.attached["m1"] == ["fake-1"])
        #expect(duplicates == [["m1", "s1"]])
        #expect(model.state?.sessions.map(\.id) == ["s1", "fake-1", "s2"])
        #expect(!model.isCreateUnderWay)
        #expect(line("s1", of: model)?.isDuplicating == false)
        #expect(line("s2", of: model)?.canDuplicate == true)
    }

    @Test("a list's Duplicate that fails says why, opens and claims nothing, and frees the next create")
    internal func listDuplicateFails() async throws {
        defer { world.remove() }
        try await world.attach([], of: ["s1"])
        let model = list()
        world.core.failNext("duplicateSession", with: Self.refused)
        await model.duplicate(sessionId: "s1") { _ in Issue.record("a failed Duplicate opened") }?.value
        #expect(model.notice == Self.refused.reason)
        #expect(graph.panes.claims.isEmpty)
        #expect((graph.connections.attached["m1"] ?? []).isEmpty)
        #expect(!model.isCreateUnderWay)
    }

    @Test("a list's Duplicate whose screen was left stays attached, opening and claiming nothing")
    internal func listDuplicateLeft() async throws {
        defer { world.remove() }
        try await world.attach([], of: ["s1"])
        let model = list()
        let gate = world.core.hold("duplicateSession")
        let duplicating = model.duplicate(sessionId: "s1") { _ in Issue.record("a left screen opened its copy") }
        await gate.reached()
        model.setShowing(false)
        gate.open()
        await duplicating?.value
        #expect(graph.connections.attached["m1"] == ["fake-1"])
        #expect(graph.panes.claims.isEmpty)
    }

    @Test("a New session under way holds a list's Duplicate")
    internal func newSessionHoldsDuplicate() async throws {
        defer { world.remove() }
        try await world.attach([], of: ["s1"])
        let model = list()
        let gate = world.core.hold("createSession")
        let creating = model.createAndOpen { _ in
            // the window would open it
        }
        await gate.reached()
        #expect(line("s1", of: model)?.canDuplicate == false)
        #expect(view().canDuplicate == false)
        #expect(model.duplicate(sessionId: "s1") { _ in Issue.record("a Duplicate began") } == nil)
        gate.open()
        await creating?.value
        #expect(duplicates.isEmpty)
        #expect(line("s1", of: model)?.canDuplicate == true)
    }

    @Test("the session view's Duplicate says so in its title, then opens the copy in control")
    internal func viewDuplicateLands() async throws {
        defer { world.remove() }
        try await world.attach()
        let model = view()
        let gate = world.core.hold("duplicateSession")
        var opened: [SessionKey] = []
        let duplicating = model.duplicate { opened.append($0) }
        await gate.reached()
        #expect(model.isDuplicating)
        #expect(!model.canDuplicate)
        #expect(model.duplicate { _ in Issue.record("a second create began") } == nil)
        gate.open()
        await duplicating?.value
        #expect(opened == [Self.copy])
        #expect(graph.panes.claims == [Self.copy])
        #expect(graph.connections.attached["m1"] == ["s1", "fake-1"])
        #expect(duplicates == [["m1", "s1"]])
        #expect(!model.isDuplicating)
        #expect(model.canDuplicate)
    }

    @Test("the session view's Duplicate that fails says why in its banner and opens nothing")
    internal func viewDuplicateFails() async throws {
        defer { world.remove() }
        try await world.attach()
        let model = view()
        world.core.failNext("duplicateSession", with: Self.refused)
        await model.duplicate { _ in Issue.record("a failed Duplicate opened") }?.value
        #expect(model.notice == Self.refused.reason)
        #expect(graph.panes.claims.isEmpty)
        #expect(graph.connections.attached["m1"] == ["s1"])
        #expect(model.canDuplicate)
    }

    @Test("a copy whose attach fails says why in the session view's banner and opens nothing")
    internal func viewCopyAttachFails() async throws {
        defer { world.remove() }
        try await world.attach()
        let model = view()
        world.core.failNext("attachSession", with: Self.refused)
        await model.duplicate { _ in Issue.record("a copy not attached opened") }?.value
        #expect(model.notice == Self.refused.reason)
        #expect(graph.panes.claims.isEmpty)
        #expect(graph.connections.attached["m1"] == ["s1"])
    }

    @Test("a session view gone before its copy landed leaves the copy attached, unopened and unclaimed")
    internal func viewGoneMeanwhile() async throws {
        defer { world.remove() }
        try await world.attach()
        let model = view()
        let gate = world.core.hold("attachSession")
        let duplicating = model.duplicate { _ in Issue.record("a view gone opened its copy") }
        await gate.reached()
        model.setShowing(false)
        gate.open()
        await duplicating?.value
        #expect(graph.connections.attached["m1"] == ["s1", "fake-1"])
        #expect(graph.panes.claims.isEmpty)
    }
}
