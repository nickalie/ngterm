import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// A machine's sessions on the fake core, as mobile/android's `SessionsViewModelTest` has them.
@MainActor
@Suite("a machine's sessions")
internal struct SessionsModelTests {
    private static let missingCli = "/bin/sh: 1: ngtermctl: not found"

    private let test = TestGraph()

    private func model() -> SessionsModel {
        SessionsModel(graph: test.graph, machineId: "m1")
    }

    // the machine m1, saved, with `sessions` there
    private func seeded(_ sessions: [String] = []) throws {
        try test.seed(TestGraph.machine(keyId: "k1"))
        test.core.sessions["m1"] = sessions.map { fakeSession(id: $0) }
    }

    // m1 with `sessions`, connected (and listed) before the screen opens
    private func connected(_ sessions: [String] = []) async throws {
        try seeded(sessions)
        await test.connections.connect(TestGraph.machine(keyId: "k1").core).value
        await test.settled()
    }

    // the ids of the machine's listing; none before it lists them
    private func ids(_ model: SessionsModel) -> [String] {
        model.state?.sessions.map(\.id) ?? []
    }

    private func lists() -> Int {
        test.core.calls.count { $0.method == "listSessions" }
    }

    @Test("opening a connected machine lists it afresh, once")
    internal func openingLists() async throws {
        defer { test.remove() }
        try await connected(["s1"])
        let before = lists()
        test.core.sessions["m1"]?.append(fakeSession(id: "s2"))
        let model = model()
        await model.opened()?.value
        #expect(model.opened() == nil)
        await test.settled()
        #expect(ids(model) == ["s1", "s2"])
        #expect(lists() == before + 1)
    }

    @Test("a machine not connected is listed by its connect")
    internal func connectLists() async throws {
        defer { test.remove() }
        try seeded(["s1"])
        let model = model()
        #expect(model.opened() == nil)
        #expect(model.state?.link == .offline)
        #expect(SessionsBody(try #require(model.state)) == .notConnected(isDeviceOffline: false))
        #expect(lists() == 0)
        model.connect()
        await Wait.until { ids(model) == ["s1"] }
        #expect(model.state?.link == .connected)
    }

    @Test("End session removes it, and a failure is a notice")
    internal func end() async throws {
        defer { test.remove() }
        try await connected(["s1", "s2"])
        let model = model()
        #expect(ids(model) == ["s1", "s2"])
        let ending = model.end(sessionId: "s1")
        #expect(model.state?.ending == ["s1"])
        #expect(model.end(sessionId: "s1") == nil)
        await ending?.value
        await test.settled()
        #expect(ids(model) == ["s2"])
        #expect(model.state?.ending.isEmpty == true)
        test.core.failNext("endSession", with: .Remote(reason: "no session with that id"))
        await model.end(sessionId: "s2")?.value
        #expect(model.notice == "no session with that id")
        #expect(ids(model) == ["s2"])
        model.dismissNotice()
        #expect(model.notice == nil)
    }

    @Test("attaching puts the session in the Attached group and opens it; ending it lets it go")
    internal func attach() async throws {
        defer { test.remove() }
        try await connected(["s1", "s2"])
        let model = model()
        var opened = 0
        await model.attach(sessionId: "s2") { opened += 1 }.value
        #expect(opened == 1)
        #expect(model.state?.attached == ["s2"])
        guard case .sessions(let attached, let others, _) = SessionsBody(try #require(model.state)) else {
            Issue.record("not a list")
            return
        }
        #expect(attached.map(\.id) == ["s2"])
        #expect(others.map(\.id) == ["s1"])
        model.detach(sessionId: "s2")
        #expect(model.state?.attached.isEmpty == true)
        await model.attach(sessionId: "s1") { opened += 1 }.value
        await model.end(sessionId: "s1")?.value
        await test.settled()
        #expect(model.state?.attached.isEmpty == true)
        #expect(test.core.calls.count { $0.method == "detachSession" && $0.args[1] as? String == "s1" } == 1)
    }

    @Test("an attach that fails opens nothing, and says why")
    internal func attachFails() async throws {
        defer { test.remove() }
        try await connected(["s1"])
        let model = model()
        test.core.failNext("attachSession", with: .Remote(reason: "the session is gone"))
        var opened = false
        await model.attach(sessionId: "s1") { opened = true }.value
        #expect(!opened)
        #expect(model.notice == "the session is gone")
        #expect(model.state?.attached.isEmpty == true)
    }

    @Test("a missing ngtermctl and a failed refresh are shown")
    internal func missingCliAndFailedRefresh() async throws {
        defer { test.remove() }
        try seeded()
        test.core.failNext("listSessions", with: .Remote(reason: Self.missingCli))
        let model = model()
        model.connect()
        await Wait.until { model.state?.failure != nil }
        #expect(SessionsBody(try #require(model.state)) == .missingCli(reason: Self.missingCli))
        // installed now: Retry lists it
        await model.refresh()
        await test.settled()
        #expect(model.state?.failure == nil)
        #expect(SessionsBody(try #require(model.state)) == .empty(failure: nil))
        test.core.failNext("listSessions", with: .Remote(reason: "build-01 gave no answer"))
        await model.refresh()
        #expect(SessionsBody(try #require(model.state)) == .empty(failure: "build-01 gave no answer"))
        #expect(!model.isRefreshing)
    }

    @Test("a removed machine is gone")
    internal func removed() async throws {
        defer { test.remove() }
        try await connected(["s1"])
        let model = model()
        #expect(model.state != nil)
        try test.graph.machines.delete(id: "m1")
        #expect(model.state == nil)
    }
}
