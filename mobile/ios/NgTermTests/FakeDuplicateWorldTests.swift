import Foundation
import NgtermCore
import NgtermCoreFakes
import NgtermFeatures
import Testing

@testable import NgTerm

/// The fake world's Duplicate, which the UI tests rely on: its copy shows a pane another device leads until this one
/// takes control, the first Duplicate fails when the launch asks, and each one is held a while when asked.
@MainActor
@Suite("the fake world's Duplicate")
internal struct FakeDuplicateWorldTests {
    private static let pause = Duration.milliseconds(300)
    private static let size = PaneSize(cols: 40, rows: 12, cellWidth: 8, cellHeight: 16)

    private let work = AppWindow.work

    // the fake session world launched with `arguments` too, `split` connected, Duplicates held `pause` when asked
    private func connected(_ arguments: [String]) async throws -> (AppGraph, FakeMobileCore) {
        let graph = FakeWorld.graph(
            arguments: [CoreChoice.fakeArgument, CoreChoice.fakeKeyArgument, CoreChoice.fakeSessionArgument]
                + arguments,
            duplicatePause: Self.pause
        )
        let core = try #require(graph.connections.core as? FakeMobileCore)
        graph.start()
        try await AppWindow.until { graph.connections.links[work.machineId] == .connected }
        return (graph, core)
    }

    private func duplicateWork(_ core: FakeMobileCore) async throws -> RemoteSession {
        try await core.duplicateSession(machineId: work.machineId, sessionId: work.sessionId)
    }

    // the roles the core reported for `pane`, oldest first
    private func leads(_ core: FakeMobileCore, of pane: PaneRef) -> [PaneRole] {
        core.emitted.compactMap { event in
            guard case .paneLead(let led, let role, _) = event, led == pane else {
                return nil
            }
            return role
        }
    }

    @Test("the copy attaches with no lead, its pane is led elsewhere as it shows, and here once taken")
    internal func copyLead() async throws {
        let (graph, core) = try await connected([])
        let copy = try await duplicateWork(core)
        #expect(copy.id == FakeSessionWorld.copyId)
        let pane = PaneRef(machineId: work.machineId, sessionId: copy.id, paneId: "P1")
        _ = try await core.listSessions(machineId: work.machineId)
        try await core.attachSession(machineId: work.machineId, sessionId: copy.id)
        #expect(leads(core, of: pane).isEmpty)
        let shown = try await core.showPane(
            machineId: work.machineId,
            sessionId: copy.id,
            paneId: pane.paneId,
            size: Self.size
        )
        #expect(leads(core, of: pane) == [.follower])
        let screen = try shown.frame(sinceSeq: 0)
        #expect(screen.contains(Data("P1 ready".utf8)), "the copy's pane names itself")
        try core.takeLead(machineId: work.machineId, sessionId: copy.id, paneId: pane.paneId)
        #expect(leads(core, of: pane) == [.follower, .leader])
        #expect(graph.connections.links[work.machineId] == .connected)
    }

    @Test("the first Duplicate fails with the world's refusal when asked, and the next one lands")
    internal func firstFails() async throws {
        let (graph, core) = try await connected([CoreChoice.fakeDuplicateFailsArgument])
        await #expect(throws: MobileError.Remote(reason: FakeSessionWorld.duplicateRefusal)) {
            _ = try await duplicateWork(core)
        }
        #expect(try await duplicateWork(core).id == FakeSessionWorld.copyId)
        #expect(graph.connections.links[work.machineId] == .connected)
    }

    @Test("with no ask the first Duplicate lands at once")
    internal func firstLands() async throws {
        let (graph, core) = try await connected([])
        let clock = ContinuousClock()
        let started = clock.now
        #expect(try await duplicateWork(core).id == FakeSessionWorld.copyId)
        #expect(clock.now - started < FakeWorld.duplicatePause)
        #expect(graph.connections.links[work.machineId] == .connected)
    }

    @Test("each Duplicate is held its pause once under way when asked, then lands")
    internal func held() async throws {
        let (graph, core) = try await connected([CoreChoice.fakeDuplicateSlowArgument])
        let clock = ContinuousClock()
        for number in 1...2 {
            let started = clock.now
            #expect(try await duplicateWork(core).id == "fake-\(number)")
            #expect(clock.now - started >= Self.pause, "Duplicate \(number) was held")
        }
        #expect(graph.connections.links[work.machineId] == .connected)
    }
}
