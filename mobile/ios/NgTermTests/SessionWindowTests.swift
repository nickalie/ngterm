import NgtermCore
import NgtermFeatures
import Testing
import UIKit

@testable import NgTerm

/// A session's own window (iPad) on the fake world's session `work`: it holds the session while it shows, so no main
/// window draws it too, shows its panes, and lets it go as it closes.
@MainActor
@Suite("a session's own window")
internal struct SessionWindowTests {
    @Test("while it shows, the session is drawn there alone, its panes shown; closing it gives it back")
    internal func holdsTheSession() async throws {
        let (graph, core) = try AppWindow.sessionWorld()
        try await AppWindow.attachWork(graph)
        let windows = SessionWindows()
        let session = SessionWindowID(AppWindow.work)
        let main = ScaffoldModel(windows: windows)
        #expect(main.open(session) == .here, "no window of its own yet")
        main.close()

        let (window, _) = try AppWindow.show(SessionWindow(graph: graph, session: session, windows: windows))
        defer { window.isHidden = true }
        try await AppWindow.until { windows.open.contains(session) }
        #expect(main.open(session) == .ownWindow, "a main window brings its window forward instead")
        #expect(main.session == nil, "and shows nothing of it")
        let pane = PaneRef(machineId: session.machineID, sessionId: session.sessionID, paneId: "P1")
        try await AppWindow.until { core.shownPanes.contains(pane) }
        #expect(AppWindow.titles(in: window).contains(FakeSessionWorld.sessionId), "the session's name is its title")

        window.rootViewController = nil
        try await AppWindow.until { !windows.open.contains(session) }
        #expect(main.open(session) == .here, "a main window shows it again")
    }

    @Test("the session view clears once the session is no longer attached")
    internal func sessionGone() async throws {
        let (graph, _) = try AppWindow.sessionWorld()
        try await AppWindow.attachWork(graph)
        let windows = SessionWindows()
        let session = SessionWindowID(AppWindow.work)
        let (window, _) = try AppWindow.show(SessionWindow(graph: graph, session: session, windows: windows))
        defer { window.isHidden = true }
        try await AppWindow.until { AppWindow.titles(in: window).contains(FakeSessionWorld.sessionId) }

        graph.connections.detach(machineId: session.machineID, sessionId: session.sessionID)
        try await AppWindow.until { !AppWindow.titles(in: window).contains(FakeSessionWorld.sessionId) }
    }
}
