import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// New session on the fake core: in one tap and from the named sheet, each landing in its session.
@MainActor
@Suite("New session")
internal struct NewSessionTests {
    private static let refused = MobileError.Remote(reason: "build-01 does not run live sessions")

    private let test = TestGraph()

    private func model() -> SessionsModel {
        let model = SessionsModel(graph: test.graph, machineId: "m1")
        model.setShowing(true)
        return model
    }

    private var claims: Set<SessionKey> {
        test.graph.panes.claims
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

    private func lists() -> Int {
        test.core.calls.count { $0.method == "listSessions" }
    }

    @Test("New session in the sheet creates it, named or not, and closes the sheet")
    internal func create() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        model.openNewSession()
        #expect(model.newSession == NewSessionState())
        await model.create(name: "  api  ") { _ in
            // opens nothing
        }?.value
        #expect(model.newSession == nil)
        model.openNewSession()
        await model.create(name: " ") { _ in
            // opens nothing
        }?.value
        #expect(model.newSession == nil)
        await test.settled()
        #expect(model.state?.sessions.map(\.name) == ["api", "session 2"])
        let names = test.core.calls.filter { $0.method == "createSession" }.map { $0.args[1] as? String }
        #expect(names == ["api", nil])
    }

    @Test("one tap on New session lands in the session it creates, attached and in this device's control")
    internal func createAndOpen() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        var opened: [String] = []
        await model.createAndOpen { opened.append($0) }?.value
        #expect(opened == ["fake-1"])
        #expect(model.newSession == nil)
        #expect(!model.isCreating)
        #expect(model.notice == nil)
        #expect(model.state?.attached == ["fake-1"])
        #expect(test.core.calls.contains(FakeCall("createSession", ["m1", String?.none])))
        let pane = PaneRef(machineId: "m1", sessionId: "fake-1", paneId: "P1")
        test.core.emit(.paneLead(pane: pane, role: .follower, covered: true))
        await test.settled()
        #expect(test.core.calls.contains(FakeCall("takeLead", ["m1", "fake-1", "P1"])))
    }

    @Test("a second tap while New session runs starts nothing; the spinner holds its place")
    internal func createAndOpenOnce() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        let gate = test.core.hold("createSession")
        var opened: [String] = []
        let creating = model.createAndOpen { opened.append($0) }
        await gate.reached()
        #expect(model.isCreating)
        #expect(model.createAndOpen { opened.append($0) } == nil)
        model.openNewSession()
        #expect(model.create(name: "api") { opened.append($0) } == nil)
        gate.open()
        await creating?.value
        #expect(opened == ["fake-1"])
        #expect(test.core.calls.count { $0.method == "createSession" } == 1)
    }

    @Test("a one-tap create that fails opens nothing, and says why")
    internal func createAndOpenFails() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        test.core.failNext("createSession", with: Self.refused)
        var opened: [String] = []
        await model.createAndOpen { opened.append($0) }?.value
        #expect(opened.isEmpty)
        #expect(!model.isCreating)
        #expect(model.notice == "build-01 does not run live sessions")
        #expect(claims.isEmpty)
    }

    @Test("a created session whose attach fails stays in the list, opens nothing and says why")
    internal func createdAttachFails() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        test.core.failNext("attachSession", with: .Remote(reason: "the session is gone"))
        var opened: [String] = []
        await model.createAndOpen { opened.append($0) }?.value
        #expect(opened.isEmpty)
        #expect(model.notice == "the session is gone")
        #expect(ids(model) == ["fake-1"])
        #expect(model.state?.attached.isEmpty == true)
        #expect(claims.isEmpty)
    }

    @Test("a created session missing from the last listing (a listing raced the create's) is listed afresh and opens")
    internal func createdNotListed() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        test.core.failNext("attachSession", with: .NotListed(sessionId: "fake-1"))
        let before = lists()
        var opened: [String] = []
        await model.createAndOpen { opened.append($0) }?.value
        #expect(opened == ["fake-1"])
        #expect(model.notice == nil)
        #expect(lists() == before + 1)
        #expect(test.core.calls.count { $0.method == "attachSession" } == 2)
    }

    @Test("Create in the named sheet lands in the session too, the sheet closed before it opens")
    internal func createNamedOpens() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        model.openNewSession()
        var sheetOnOpen: [NewSessionState?] = []
        await model.create(name: "api") { _ in sheetOnOpen.append(model.newSession) }?.value
        #expect(sheetOnOpen == [nil])
        #expect(model.state?.attached == ["fake-1"])
        #expect(claims == [SessionKey(machineId: "m1", sessionId: "fake-1")])
    }

    @Test("a named create whose sheet was cancelled only lists its session")
    internal func createNamedCancelled() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        model.openNewSession()
        let gate = test.core.hold("createSession")
        var opened = false
        let creating = model.create(name: "api") { _ in opened = true }
        await gate.reached()
        model.closeNewSession()
        gate.open()
        await creating?.value
        await test.settled()
        #expect(!opened)
        #expect(ids(model) == ["fake-1"])
        #expect(model.state?.attached.isEmpty == true)
        #expect(model.notice == nil)
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
        model.openedElsewhere()
        gate.open()
        await creating?.value
        #expect(opened.isEmpty)
        #expect(claims.isEmpty)
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

    @Test("a named create whose attach fails closes the sheet and says why in the banner")
    internal func createNamedAttachFails() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        model.openNewSession()
        test.core.failNext("attachSession", with: .Remote(reason: "the session is gone"))
        var opened = false
        await model.create(name: "api") { _ in opened = true }?.value
        #expect(!opened)
        #expect(model.newSession == nil)
        #expect(model.notice == "the session is gone")
    }

    @Test("a failed create stays in the sheet it began in")
    internal func createFails() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        model.openNewSession()
        test.core.failNext("createSession", with: Self.refused)
        await model.create(name: "api") { _ in
            // opens nothing
        }?.value
        #expect(model.newSession == NewSessionState(failure: "build-01 does not run live sessions"))
        #expect(model.notice == nil)
        model.closeNewSession()
        #expect(model.newSession == nil)
    }

    @Test("a create whose sheet closed fails to the banner, and leaves a new sheet alone")
    internal func createFailsAfterClose() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        model.openNewSession()
        test.core.failNext("createSession", with: Self.refused)
        let gate = test.core.hold("createSession")
        let creating = model.create(name: "api") { _ in
            // opens nothing
        }
        await gate.reached()
        model.closeNewSession()
        model.openNewSession()
        gate.open()
        await creating?.value
        #expect(model.notice == "build-01 does not run live sessions")
        // the sheet opened since is not the failed create's
        #expect(model.newSession == NewSessionState())
    }

    @Test("a create whose sheet closed leaves a new sheet open when it lands")
    internal func createLandsAfterClose() async throws {
        defer { test.remove() }
        try await connected()
        let model = model()
        model.openNewSession()
        let gate = test.core.hold("createSession")
        let creating = model.create(name: "api") { _ in
            // opens nothing
        }
        await gate.reached()
        model.closeNewSession()
        model.openNewSession()
        gate.open()
        await creating?.value
        await test.settled()
        #expect(model.state?.sessions.map(\.name) == ["api"])
        #expect(model.newSession == NewSessionState())
        #expect(model.notice == nil)
    }
}
