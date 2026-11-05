import NgtermCore
import NgtermCoreFakes
import NgtermPlatform
import Testing

@testable import NgtermFeatures

/// The status baseline: a turn to waiting or done notifies once, a repeat doesn't, and a snapshot's status (`changed:
/// false`) is recorded without a notice, so a refresh that finds it again says nothing; kept across launches, and let
/// go with its session's detach.
@MainActor
@Suite("the status baseline")
internal struct StatusBaselineTests {
    private let test = TestGraph()
    private let key = SessionKey(machineId: "m1", sessionId: "s1")

    private var baseline: StatusBaseline {
        test.graph.baseline
    }

    private func attach() async throws {
        try test.seed(TestGraph.machine(keyId: "k1"))
        test.core.sessions["m1"] = [fakeSession(id: "s1", name: "api")]
        await test.connections.connect(TestGraph.machine(keyId: "k1").core).value
        await test.settled()
        try await test.connections.attach(machineId: "m1", sessionId: "s1")
    }

    private func status(_ status: String?, changed: Bool) async {
        let said = status.map { word in
            RemoteStatus(status: word, blink: false, color: nil, shape: nil, pane: nil, changedAt: nil)
        }
        test.core.emit(.sessionStatus(machineId: "m1", sessionId: "s1", status: said, changed: changed))
        await test.settled()
    }

    @Test("a turn to waiting or done notifies once; a repeat or another status doesn't")
    internal func turns() {
        defer { test.remove() }
        #expect(baseline.turn(key, to: "active") == nil)
        #expect(baseline.turn(key, to: "blocked") == .waiting)
        #expect(baseline.turn(key, to: "blocked") == nil)
        #expect(baseline.turn(key, to: "completed") == .done)
        #expect(baseline.turn(key, to: nil) == nil)
        #expect(baseline.status(of: key) == "completed")
    }

    @Test("a snapshot's status is recorded with no notice, so a refresh finding it again says nothing")
    internal func snapshot() async throws {
        defer { test.remove() }
        try await attach()
        await status("blocked", changed: false)
        #expect(test.notices.posted.isEmpty)
        #expect(baseline.status(of: key) == "blocked")
        #expect(baseline.turn(key, to: "blocked") == nil)
        // a stream's clear (it went) keeps the last status seen
        await status(nil, changed: false)
        #expect(baseline.status(of: key) == "blocked")
    }

    @Test("kept across launches, and let go with its session's detach")
    internal func kept() async throws {
        defer { test.remove() }
        try await attach()
        await status("active", changed: true)
        #expect(StatusBaselineStore(directory: test.directory).statuses == ["m1": ["s1": "active"]])
        test.connections.detach(machineId: "m1", sessionId: "s1")
        #expect(baseline.status(of: key) == nil)
        #expect(StatusBaselineStore(directory: test.directory).statuses.isEmpty)
    }
}
