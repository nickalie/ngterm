import NgtermCore
import NgtermFeatures
import Testing
import UIKit

@testable import NgTerm

/// A main window over the fake world: the first run's cover while there is no key, and where a notification's tap
/// takes it (the machine, its session with the pane, a session attached only once its machine lists it), and the
/// session view letting the window go back once the session is let go.
@MainActor
@Suite("a main window")
internal struct RootViewTests {
    private static func show(
        _ graph: AppGraph,
        windows: SessionWindows = SessionWindows()
    ) throws -> (UIWindow, UIViewController) {
        try AppWindow.show(RootView(graph: graph, coreVersion: "test", windows: windows))
    }

    @Test("with no key, the first run covers the window")
    internal func firstRun() async throws {
        let graph = CoreChoice(arguments: [CoreChoice.fakeArgument]).graph
        let (window, host) = try Self.show(graph)
        defer { window.isHidden = true }
        try await AppWindow.until { host.presentedViewController != nil }
        #expect(graph.isStarted, "the window starts the app's graph")
    }

    @Test("with a key, the window opens on the machines, uncovered")
    internal func pastFirstRun() async throws {
        let (graph, _) = try AppWindow.sessionWorld()
        let (window, host) = try Self.show(graph)
        defer { window.isHidden = true }
        try await AppWindow.until { graph.connections.links[FakeSessionWorld.machineId] == .connected }
        #expect(host.presentedViewController == nil)
        #expect(!AppWindow.titles(in: window).contains(FakeSessionWorld.machineId), "no machine's sessions show")
    }

    @Test("a machine picked in the list shows its sessions")
    internal func pickMachine() async throws {
        let (graph, _) = try AppWindow.sessionWorld()
        let (window, _) = try Self.show(graph)
        defer { window.isHidden = true }
        if window.traitCollection.horizontalSizeClass == .regular {
            // an iPad held upright hides the machines behind the sidebar button: on its side, they show
            let size = window.bounds.size
            window.frame = CGRect(x: 0, y: 0, width: max(size.width, size.height), height: min(size.width, size.height))
        }
        var machines: UICollectionView?
        try await AppWindow.until {
            window.layoutIfNeeded()
            machines = AppWindow.views(UICollectionView.self, in: window).first { list in
                list.numberOfSections > 0 && list.numberOfItems(inSection: 0) > 0
            }
            return machines != nil
        }
        let list = try #require(machines)
        #expect(!AppWindow.titles(in: window).contains(FakeSessionWorld.machineId))

        list.delegate?.collectionView?(list, didSelectItemAt: IndexPath(item: 0, section: 0))
        try await AppWindow.until { AppWindow.titles(in: window).contains(FakeSessionWorld.machineId) }
    }

    @Test("a program's clipboard read asks over the window")
    internal func clipboardRead() async throws {
        let graph = CoreChoice(arguments: [
            CoreChoice.fakeArgument, CoreChoice.fakeKeyArgument, CoreChoice.fakeClipboardReadArgument,
        ]).graph
        let (window, host) = try Self.show(graph)
        defer { window.isHidden = true }
        // The first read can present its sheet before the event stream delivers the second.
        try await AppWindow.until { graph.clipboardReads.pending.count == 2 }
        try await AppWindow.until { host.presentedViewController != nil }
        #expect(graph.clipboardReads.pending.map(\.pane.sessionId) == ["docs-site", "notes"])
    }

    @Test("the question about notifying asks over the window as the first session is attached")
    internal func notificationQuestion() async throws {
        let graph = CoreChoice(arguments: [
            CoreChoice.fakeArgument, CoreChoice.fakeKeyArgument, CoreChoice.fakeSessionArgument,
            CoreChoice.fakeNotificationQuestionArgument,
        ]).graph
        let (window, host) = try Self.show(graph)
        defer { window.isHidden = true }
        try await AppWindow.until { graph.connections.links[FakeSessionWorld.machineId] == .connected }
        #expect(host.presentedViewController == nil, "nothing is attached yet")

        try await graph.connections.attach(machineId: AppWindow.work.machineId, sessionId: AppWindow.work.sessionId)
        try await AppWindow.until { host.presentedViewController != nil }
        #expect(graph.notificationPrefs.prefs.isPermissionAsked, "asked once")
    }

    @Test("a tap on a machine's notification shows its sessions")
    internal func tapMachine() async throws {
        let (graph, _) = try AppWindow.sessionWorld()
        let target = OpenTarget(machineId: FakeSessionWorld.machineId)
        graph.openRequests.request(target)
        let (window, _) = try Self.show(graph)
        defer { window.isHidden = true }
        try await AppWindow.until { AppWindow.titles(in: window).contains(FakeSessionWorld.machineId) }
        #expect(graph.openRequests.pending == nil, "the window took the request")
    }

    @Test("a tap on an attached session's notification shows the session with its pane selected")
    internal func tapAttachedSession() async throws {
        let (graph, core) = try AppWindow.sessionWorld()
        try await AppWindow.attachWork(graph)
        let (window, _) = try Self.show(graph)
        defer { window.isHidden = true }
        let work = AppWindow.work
        graph.openRequests.request(OpenTarget(machineId: work.machineId, sessionId: work.sessionId, paneId: "P2"))
        try await AppWindow.until { AppWindow.titles(in: window).contains(work.sessionId) }
        #expect(graph.panes.focus[work] == "P2")
        let pane = PaneRef(machineId: work.machineId, sessionId: work.sessionId, paneId: "P2")
        try await AppWindow.until { core.shownPanes.contains(pane) }
        #expect(graph.openRequests.pending == nil)
    }

    @Test("a tap on a session not attached attaches it once its machine lists it, then shows it")
    internal func tapSessionNotAttached() async throws {
        let (graph, core) = try AppWindow.sessionWorld()
        let work = AppWindow.work
        graph.openRequests.request(OpenTarget(machineId: work.machineId, sessionId: work.sessionId))
        let (window, _) = try Self.show(graph)
        defer { window.isHidden = true }
        try await AppWindow.until { AppWindow.titles(in: window).contains(work.sessionId) }
        #expect(graph.connections.attached[work.machineId] == [work.sessionId])
        #expect(core.calls.count { $0.method == "attachSession" } == 1)
    }

    @Test("a tap on a session gone from its machine leaves the machine showing")
    internal func tapSessionGone() async throws {
        let (graph, core) = try AppWindow.sessionWorld()
        let windows = SessionWindows()
        // Another main window still has this session's gone screen open. A rejected tap must not take it over:
        // unlike a navigation title, this also detects opening the title-less gone screen by mistake.
        let other = ScaffoldModel(windows: windows)
        let gone = SessionWindowID(machineID: FakeSessionWorld.machineId, sessionID: "gone")
        #expect(other.open(gone) == .here)
        graph.openRequests.request(OpenTarget(machineId: FakeSessionWorld.machineId, sessionId: "gone"))
        let (window, _) = try Self.show(graph, windows: windows)
        defer { window.isHidden = true }
        try await AppWindow.until { AppWindow.titles(in: window).contains(FakeSessionWorld.machineId) }
        try await AppWindow.until { graph.connections.trees[FakeSessionWorld.machineId] != nil }
        // The listing wakes attachWhenListed's Observations asynchronously; allow its verdict and the resulting
        // navigation/layout to run before checking that the window stayed on the machine.
        for _ in 0..<10 {
            try await Task.sleep(for: .milliseconds(20))
            window.layoutIfNeeded()
        }
        #expect(!core.calls.contains { $0.method == "attachSession" }, "a missing session is never attached")
        #expect(AppWindow.titles(in: window).contains(FakeSessionWorld.machineId))
        #expect(other.shownSessionPlacement == .here, "the tap never opened the missing session")
    }

    @Test("the session let go elsewhere leaves the window on its machine's sessions")
    internal func sessionLetGo() async throws {
        let (graph, _) = try AppWindow.sessionWorld()
        try await AppWindow.attachWork(graph)
        let (window, _) = try Self.show(graph)
        defer { window.isHidden = true }
        let work = AppWindow.work
        graph.openRequests.request(OpenTarget(machineId: work.machineId, sessionId: work.sessionId))
        try await AppWindow.until { AppWindow.titles(in: window).contains(work.sessionId) }

        graph.connections.detach(machineId: work.machineId, sessionId: work.sessionId)
        try await AppWindow.until { !AppWindow.titles(in: window).contains(work.sessionId) }
        #expect(AppWindow.titles(in: window).contains(work.machineId))
    }
}
