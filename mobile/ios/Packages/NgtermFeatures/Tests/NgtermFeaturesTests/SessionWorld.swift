import NgtermCore
import NgtermCoreFakes

@testable import NgtermFeatures

/// A ``TestGraph`` with build-01 (`m1`) connected and its session `api` (`s1`) attached, for the session view's tests:
/// its stream's events emitted at will, its panes `P1` and `P2` drawing a line that names them.
@MainActor
internal final class SessionWorld {
    /// The session.
    internal static let key = SessionKey(machineId: "m1", sessionId: "s1")
    /// The grid a test shows a pane at.
    internal static let size = PaneSize(cols: sizeColumns, rows: sizeRows, cellWidth: cellWidth, cellHeight: cellHeight)
    /// The panes of a split.
    internal static let panes = ["P1", "P2"]
    nonisolated private static let columns = 60
    nonisolated private static let rows = 30
    // the rows of filler under a pane's prompt, and its prompt's row
    nonisolated private static let filler = 4..<8
    nonisolated private static let promptRow = 2
    private static let sizeColumns: UInt16 = 40
    private static let sizeRows: UInt16 = 12
    private static let cellWidth: UInt32 = 16
    private static let cellHeight: UInt32 = 32

    /// The graph.
    internal let test = TestGraph()

    /// The app's graph.
    internal var graph: AppGraph {
        test.graph
    }

    /// The fake core.
    internal var core: FakeMobileCore {
        test.core
    }

    internal init() {
        test.core.paneFactory = { _, _, paneId, _ in
            FakePane(paneId: paneId) { since in
                var writer = RowBufferWriter(columns: Self.columns, rows: Self.rows, seq: since + 1)
                    .text(0, "\(paneId) $ make test")
                    .text(1, "ok  \(paneId == "P1" ? "parser" : "lexer")  0.42s")
                    .text(Self.promptRow, "$ ")
                for row in Self.filler {
                    writer = writer.text(row, "  line \(row) of \(paneId)")
                }
                return writer.bytes()
            }
        }
    }

    /// The pane `paneId` of the session.
    internal static func pane(_ paneId: String) -> PaneRef {
        PaneRef(machineId: key.machineId, sessionId: key.sessionId, paneId: paneId)
    }

    /// A layout of `panes` on `axis`, `shown` or hidden.
    internal static func layout(_ panes: [String], axis: String?, shown: Bool) -> RemoteLayout {
        RemoteLayout(panes: panes, primary: panes.first ?? "P1", axis: axis, shown: shown)
    }

    /// A split of ``panes`` on `axis`, shown.
    internal static func split(_ axis: String) -> RemoteLayout {
        layout(panes, axis: axis, shown: true)
    }

    /// build-01 connected with its sessions `sessions` listed (`s1` is "api"), and `attach` of them attached.
    internal func attach(_ attach: [String], of sessions: [String]) async throws {
        try await self.attach(attach, of: sessions, peer: nil)
    }

    /// build-01 connected with its sessions `sessions` listed (`s1` is "api") as the app `peer` lists them (`nil`: the
    /// listing doesn't say), and `attach` of them attached.
    internal func attach(_ attach: [String], of sessions: [String], peer: PeerKind?) async throws {
        try test.seed(TestGraph.machine(keyId: "k1"))
        let count = UInt32(Self.panes.count)
        core.sessions["m1"] = sessions.map { sessionId in
            fakeSession(id: sessionId, name: sessionId == "s1" ? "api" : sessionId, paneCount: count, peer: peer)
        }
        await test.connections.connect(TestGraph.machine(keyId: "k1").core).value
        await test.settled()
        for sessionId in attach {
            try await test.connections.attach(machineId: "m1", sessionId: sessionId)
        }
    }

    /// build-01 connected with `api` (`s1`) listed and attached.
    internal func attach() async throws {
        try await attach(["s1"], of: ["s1"])
    }

    /// The session's stream says it is connected with `layout`, its panes led by this device.
    internal func stream(_ layout: RemoteLayout) async {
        await emit(
            .sessionConnection(machineId: "m1", sessionId: "s1", state: .connected),
            .sessionLayout(machineId: "m1", sessionId: "s1", layout: layout)
        )
        for paneId in layout.panes {
            await emit(.paneLead(pane: Self.pane(paneId), role: .leader, covered: false))
        }
    }

    /// Hands the core's `events` to the app, and waits for them to land.
    internal func emit(_ events: MobileEvent...) async {
        for event in events {
            core.emit(event)
        }
        await test.settled()
    }

    /// Shows the panes `paneIds` at ``size``, and waits until they are.
    internal func show(_ paneIds: String...) async {
        for paneId in paneIds {
            graph.panes.show(Self.pane(paneId), size: Self.size)
        }
        await Wait.until { paneIds.allSatisfy { graph.panes.shown[Self.pane($0)] != nil } }
    }

    /// Deletes the graph's directory.
    internal func remove() {
        test.remove()
    }

    deinit {
        // the directory goes with remove()
    }
}
