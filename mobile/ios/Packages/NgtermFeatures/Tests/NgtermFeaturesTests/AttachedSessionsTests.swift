import Foundation
import NgtermCoreFakes
import NgtermPlatform
import Testing

@testable import NgtermFeatures

/// The attached sessions across machines, as the session view, its moves and the sidebar list them, and a pane's
/// cover and the screen kept awake.
@MainActor
@Suite("the attached sessions")
internal struct AttachedSessionsTests {
    private let world = SessionWorld()

    private var attached: AttachedSessions {
        world.graph.attachedSessions
    }

    @Test("a machine's attached sessions come in its listing's order, and keep the name they were last shown with")
    internal func order() async throws {
        defer { world.remove() }
        try await world.attach(["s3", "s1"], of: ["s1", "s2", "s3"])
        #expect(attached.groups.map(\.machine) == ["build-01"])
        #expect(attached.sessions.map(\.key.sessionId) == ["s1", "s3"])
        world.graph.connections.detach(machineId: "m1", sessionId: "s1")
        #expect(attached.sessions.map(\.key.sessionId) == ["s3"])
        #expect(attached.lastSeen(SessionWorld.key)?.name == "api")
    }

    @Test("the listing's status and context stand in until the stream says its own, a clear included")
    internal func statusAndContext() async throws {
        defer { world.remove() }
        var listed = fakeSession(id: "s1", name: "api")
        listed.status = "blocked"
        listed.statusColor = "#d08770"
        listed.context = "Fix the parser"
        try world.test.seed(TestGraph.machine(keyId: "k1"))
        world.core.sessions["m1"] = [listed]
        await world.test.connections.connect(TestGraph.machine(keyId: "k1").core).value
        await world.test.settled()
        try await world.test.connections.attach(machineId: "m1", sessionId: "s1")
        var session = try #require(attached.session(SessionWorld.key))
        #expect(session.status == AgentStatus(text: "blocked", color: "#d08770"))
        #expect(session.context == "Fix the parser")

        await world.emit(
            .sessionStatus(machineId: "m1", sessionId: "s1", status: nil, changed: true),
            .sessionContext(machineId: "m1", sessionId: "s1", context: "Ship it")
        )
        session = try #require(attached.session(SessionWorld.key))
        #expect(session.status == nil)
        #expect(session.context == "Ship it")
        #expect(session.link == .connected)
    }

    @Test("a session not attached, or whose machine is gone, isn't one")
    internal func notAttached() async throws {
        defer { world.remove() }
        try await world.attach([], of: ["s1"])
        #expect(attached.session(SessionWorld.key) == nil)
        #expect(attached.groups.isEmpty)
    }

    @Test("a pane's cover: its failure, no terminal, its end, then a follower's or an attach's")
    internal func covers() {
        let leader = PaneLive(role: .leader)
        #expect(PaneCover.of(leader, hasTerminal: true, failure: nil) == nil)
        #expect(PaneCover.of(leader, hasTerminal: true, failure: "busy") == .failed(reason: "busy"))
        #expect(PaneCover.of(nil, hasTerminal: false, failure: nil) == .attaching)
        #expect(PaneCover.of(PaneLive(end: .lost(reason: "eof")), hasTerminal: true, failure: nil) == .reconnecting)
        let exited = PaneLive(end: .exited(code: 2, signal: nil))
        #expect(PaneCover.of(exited, hasTerminal: true, failure: nil) == .exited(code: 2, signal: nil))
        let follower = PaneLive(role: .follower, isCovered: true)
        #expect(PaneCover.of(follower, hasTerminal: true, failure: nil) == .follower)
        #expect(PaneCover.of(PaneLive(isCovered: true), hasTerminal: true, failure: nil) == .attaching)
        #expect(PaneCover.follower.veils)
        #expect(!PaneCover.reconnecting.veils)
    }

    @Test("the screen stays awake while any session view asks, and sleeps again once none does")
    internal func screenAwake() {
        var applied: [Bool] = []
        let awake = ScreenAwake { applied.append($0) }
        let first = UUID()
        let second = UUID()
        awake.hold(first, wanted: false)
        awake.hold(first, wanted: true)
        awake.hold(second, wanted: true)
        awake.hold(first, wanted: false)
        #expect(awake.isAwake)
        awake.hold(second, wanted: false)
        #expect(!awake.isAwake)
        #expect(applied == [true, false])
    }
}
