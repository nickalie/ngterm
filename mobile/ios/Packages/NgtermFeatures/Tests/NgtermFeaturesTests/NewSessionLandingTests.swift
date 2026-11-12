import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// Whether a create under way still lands: not once its screen is left, its sheet cancelled or another session opened.
@MainActor
@Suite("New session: landing")
internal struct NewSessionLandingTests {
    private let test = TestGraph()
    private let windows = SessionWindows()

    private var claims: Set<SessionKey> {
        test.graph.panes.claims
    }

    private func model() -> SessionsModel {
        let model = SessionsModel(graph: test.graph, machineId: "m1", windows: windows)
        model.setShowing(true)
        return model
    }

    private func connected() async throws {
        try test.seed(TestGraph.machine(keyId: "k1"))
        test.core.sessions["m1"] = []
        await test.connections.connect(TestGraph.machine(keyId: "k1").core).value
        await test.settled()
    }

    private func ids(_ model: SessionsModel) -> [String] {
        model.state?.sessions.map(\.id) ?? []
    }

    @Test("a named create whose sheet is cancelled during the attach opens nothing, claims nothing and is only listed")
    internal func createNamedCancelledAttaching() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        model.openNewSession()
        let gate = test.core.hold("attachSession")
        var opened = false
        let creating = model.create(name: "api") { _ in opened = true }
        await gate.reached()
        model.closeNewSession()
        gate.open()
        await creating?.value
        await test.settled()
        #expect(!opened)
        #expect(claims.isEmpty)
        #expect(ids(model) == ["fake-1"])
        #expect(model.state?.attached.isEmpty == true)
        #expect(model.newSession == nil)
    }

    @Test("a create whose screen was left before it attached stays attached, opens nothing and claims nothing")
    internal func createLeft() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        let gate = test.core.hold("attachSession")
        var opened: [String] = []
        let creating = model.createAndOpen { opened.append($0) }
        await gate.reached()
        model.setShowing(false)
        gate.open()
        await creating?.value
        #expect(opened.isEmpty)
        #expect(claims.isEmpty)
        #expect(model.state?.attached == ["fake-1"])
        #expect(!model.isCreating)
    }

    @Test("a create during which the window opened another session leaves that session in its place")
    internal func createOpenedElsewhere() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        let gate = test.core.hold("createSession")
        var opened: [String] = []
        let creating = model.createAndOpen { opened.append($0) }
        await gate.reached()
        model.windowShows("s0")
        gate.open()
        await creating?.value
        #expect(opened.isEmpty)
        #expect(claims.isEmpty)
    }

    @Test("a create whose window lets the session it showed go meanwhile still lands")
    internal func createShownClosed() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        model.windowShows("s0")
        let gate = test.core.hold("attachSession")
        var opened: [String] = []
        let creating = model.createAndOpen { opened.append($0) }
        await gate.reached()
        model.windowShows(nil)
        gate.open()
        await creating?.value
        #expect(opened == ["fake-1"])
        #expect(claims == [SessionKey(machineId: "m1", sessionId: "fake-1")])
    }

    @Test("a named create cancelled during the attach is only listed, though the window opened another session")
    internal func createNamedCancelledOpenedElsewhere() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        model.openNewSession()
        let gate = test.core.hold("attachSession")
        let creating = model.create(name: "api") { _ in
            // opens nothing
        }
        await gate.reached()
        model.closeNewSession()
        model.windowShows("s0")
        gate.open()
        await creating?.value
        #expect(model.state?.attached.isEmpty == true)
    }

    @Test("a named create cancelled during the attach stays attached where another window shows it")
    internal func createNamedCancelledShownElsewhere() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        model.openNewSession()
        let gate = test.core.hold("attachSession")
        let creating = model.create(name: "api") { _ in
            // opens nothing
        }
        await gate.reached()
        model.closeNewSession()
        let other = ScaffoldModel(windows: windows)
        other.session = SessionWindowID(machineID: "m1", sessionID: "fake-1")
        gate.open()
        await creating?.value
        #expect(model.state?.attached == ["fake-1"])
        #expect(!test.core.calls.contains { $0.method == "detachSession" })
        #expect(other.shownSessionPlacement == .here)
    }

    @Test("a named create cancelled during the attach stays attached where another window is opening it")
    internal func createNamedCancelledOpeningElsewhere() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        model.openNewSession()
        let gate = test.core.hold("attachSession")
        let creating = model.create(name: "api") { _ in
            // opens nothing
        }
        await gate.reached()
        await test.settled()
        model.closeNewSession()
        let other = ScaffoldModel(windows: windows)
        let session = SessionWindowID(machineID: "m1", sessionID: "fake-1")
        let opening = SessionsModel(graph: test.graph, machineId: "m1", windows: windows).attach(sessionId: "fake-1") {
            other.session = session
        }
        let key = SessionKey(machineId: "m1", sessionId: "fake-1")
        await Wait.until { test.connections.attaching[key] == 2 }
        gate.open()
        await creating?.value
        await opening.value
        #expect(other.session == session)
        #expect(model.state?.attached == ["fake-1"])
        #expect(!test.core.calls.contains { $0.method == "detachSession" })
    }

    @Test("a named create whose screen was left meanwhile closes its sheet, opens nothing and claims nothing")
    internal func createNamedLeft() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        model.openNewSession()
        let gate = test.core.hold("attachSession")
        var opened = false
        let creating = model.create(name: "api") { _ in opened = true }
        await gate.reached()
        model.setShowing(false)
        gate.open()
        await creating?.value
        #expect(!opened)
        #expect(claims.isEmpty)
        #expect(model.newSession == nil)
    }
}
