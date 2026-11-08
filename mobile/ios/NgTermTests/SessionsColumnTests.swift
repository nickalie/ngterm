import NgtermFeatures
import SwiftUI
import Testing
import UIKit

@testable import NgTerm

/// A main window's sessions column on the fake world's `split`: its list's selection is the session the window shows,
/// a session picked there is attached and shown in the window, and the machine removed leaves the window.
@MainActor
@Suite("a main window's sessions column")
internal struct SessionsColumnTests {
    // the column of `split` over `graph` in a window, navigating `scaffold`
    private static func show(_ graph: AppGraph, _ scaffold: ScaffoldModel) throws -> UIWindow {
        let column = SessionsColumn(
            graph: graph,
            machineId: FakeSessionWorld.machineId,
            scaffold: scaffold,
            machines: MachinesModel(graph: graph)
        )
        return try AppWindow.show(NavigationStack { column }).0
    }

    // the list of the column in `window` once it lists `work`, and the index path of `work` in it
    private static func list(in window: UIWindow) async throws -> (UICollectionView, IndexPath) {
        var found: UICollectionView?
        try await AppWindow.until {
            window.layoutIfNeeded()
            found = AppWindow.views(UICollectionView.self, in: window).first { list in
                list.numberOfSections > 0 && list.numberOfItems(inSection: list.numberOfSections - 1) > 0
            }
            return found != nil
        }
        let list = try #require(found)
        return (list, IndexPath(item: 0, section: list.numberOfSections - 1))
    }

    private static func connected() throws -> AppGraph {
        let (graph, _) = try AppWindow.sessionWorld()
        graph.start()
        return graph
    }

    @Test("a session picked in the list is attached, then shown in this window")
    internal func opens() async throws {
        let graph = try Self.connected()
        let scaffold = ScaffoldModel(windows: SessionWindows())
        scaffold.show(machine: FakeSessionWorld.machineId)
        let window = try Self.show(graph, scaffold)
        defer { window.isHidden = true }
        let (list, work) = try await Self.list(in: window)
        #expect(list.indexPathsForSelectedItems?.isEmpty ?? true, "nothing shows yet")

        list.delegate?.collectionView?(list, didSelectItemAt: work)
        try await AppWindow.until { scaffold.session == SessionWindowID(AppWindow.work) }
        #expect(graph.connections.attached[FakeSessionWorld.machineId] == [FakeSessionWorld.sessionId])
        #expect(scaffold.compactColumn == .detail, "a phone's stack goes on to the session")
    }

    @Test("the session the window shows is the list's selection, and another machine's is not")
    internal func selection() async throws {
        let graph = try Self.connected()
        let scaffold = ScaffoldModel(windows: SessionWindows())
        scaffold.show(machine: FakeSessionWorld.machineId)
        scaffold.session = SessionWindowID(machineID: "nas", sessionID: FakeSessionWorld.sessionId)
        let window = try Self.show(graph, scaffold)
        defer { window.isHidden = true }
        let (list, work) = try await Self.list(in: window)
        #expect(list.indexPathsForSelectedItems?.isEmpty ?? true, "a session of the same name on another machine")

        scaffold.session = SessionWindowID(AppWindow.work)
        try await AppWindow.until { list.indexPathsForSelectedItems == [work] }
    }

    @Test("the machine removed while its sessions show leaves the window")
    internal func machineGone() async throws {
        let graph = try Self.connected()
        let scaffold = ScaffoldModel(windows: SessionWindows())
        scaffold.show(machine: FakeSessionWorld.machineId)
        let window = try Self.show(graph, scaffold)
        defer { window.isHidden = true }
        _ = try await Self.list(in: window)

        try graph.machines.delete(id: FakeSessionWorld.machineId)
        try await AppWindow.until { scaffold.machine == nil }
        #expect(scaffold.compactColumn == .sidebar, "a phone's stack goes back to the machines")
    }
}
