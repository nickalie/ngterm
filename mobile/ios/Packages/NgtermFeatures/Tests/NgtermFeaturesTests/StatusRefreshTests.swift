import NgtermCore
import NgtermCoreFakes
import NgtermPlatform
import Testing

@testable import NgtermFeatures

/// The background refresh on the fake core: on a core of its own, it connects each machine of the attached set, lists
/// it, notifies the attached agents that turned waiting or done against the baseline, and disconnects; it never
/// attaches, and the app's core (suspended, its machines set aside) gets no call.
@MainActor
@Suite("the background refresh")
internal struct StatusRefreshTests {
    private let test = TestGraph()

    private static func listed(_ id: String, status: String?) -> RemoteSession {
        var session = fakeSession(id: id, name: "\(id)-name")
        session.status = status
        return session
    }

    // m1 connected with s1 and s2 attached (s1's agent working), then suspended as the app left
    private func attachThenSuspend() async throws {
        try test.seed(TestGraph.machine(keyId: "k1"))
        test.core.sessions["m1"] = [Self.listed("s1", status: "active"), Self.listed("s2", status: nil)]
        await test.connections.connect(TestGraph.machine(keyId: "k1").core).value
        await test.settled()
        try await test.connections.attach(machineId: "m1", sessionId: "s1")
        try await test.connections.attach(machineId: "m1", sessionId: "s2")
        let working = RemoteStatus(status: "active", blink: false, color: nil, shape: nil, pane: nil, changedAt: nil)
        test.core.emit(.sessionStatus(machineId: "m1", sessionId: "s1", status: working, changed: false))
        await test.settled()
        test.graph.suspend()
    }

    @Test("turns notify once, nothing attaches, and the app's core is left as it was")
    internal func refresh() async throws {
        defer { test.remove() }
        try await attachThenSuspend()
        let before = test.core.calls
        test.core.sessions["m1"] = [
            Self.listed("s1", status: "blocked"),
            Self.listed("s2", status: "completed"),
            Self.listed("s3", status: "blocked"),
        ]
        await test.graph.refresh.run()
        #expect(test.core.calls == before, "the app's core keeps what its suspend set aside")
        #expect(test.refreshCore.calls.map(\.method) == ["connect", "listSessions", "disconnect"])
        let waiting = Alert.statusId(SessionKey(machineId: "m1", sessionId: "s1"))
        let done = Alert.statusId(SessionKey(machineId: "m1", sessionId: "s2"))
        #expect(test.notices.upIds == [waiting, done])
        #expect(test.notices.posted[waiting]?.title == "s1-name is waiting for you")
        #expect(test.notices.posted[done]?.title == "s2-name finished")

        test.notices.withdraw(test.notices.upIds)
        await test.graph.refresh.run()
        #expect(test.notices.posted.isEmpty, "the same statuses again are no turn")
        #expect(!test.refreshCore.calls.contains { $0.method == "attachSession" })
    }

    @Test("a machine whose connect would wait on the user is let go at once")
    internal func waitingOnTheUser() async throws {
        defer { test.remove() }
        try await attachThenSuspend()
        // the machine offers a host key no one pinned: its question would wait for an answer
        test.refreshCore.hostKeys = ["build-01.lan": "SHA256:fAkEnEwKeY"]
        await test.graph.refresh.run()
        #expect(test.refreshCore.calls.map(\.method) == ["connect", "disconnect", "disconnect"])
        #expect(test.notices.posted.isEmpty)
    }

    @Test("nothing attached is nothing to refresh: no core is made")
    internal func nothingAttached() async throws {
        defer { test.remove() }
        try test.seed(TestGraph.machine(keyId: "k1"))
        #expect(!test.graph.refresh.hasWork)
        await test.graph.refresh.run()
        #expect(test.refreshCore.calls.isEmpty)
    }
}
