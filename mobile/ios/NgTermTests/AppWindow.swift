import NgtermCoreFakes
import NgtermFeatures
import SwiftUI
import Testing
import UIKit

@testable import NgTerm

/// A window of the app's scene showing a view of the app's (a sheet, a cover, a list's cells and the first responder
/// need one), the fake world's graph behind it, and the waits on what the view does.
@MainActor
internal enum AppWindow {
    // how often a wait looks at its condition, and how many times: counted in looks, not in time, so a busy
    // simulator's stall leaves the view its turns to get there
    private static let stepMilliseconds = 20
    private static let looks = 500

    /// The session `work` of `split`.
    internal static var work: SessionKey {
        SessionKey(machineId: FakeSessionWorld.machineId, sessionId: FakeSessionWorld.sessionId)
    }

    /// The fake world's graph past its first run, with `split` and its session `work` (``FakeSessionWorld``), and its
    /// core.
    internal static func sessionWorld() throws -> (AppGraph, FakeMobileCore) {
        let graph = CoreChoice(arguments: [
            CoreChoice.fakeArgument, CoreChoice.fakeKeyArgument, CoreChoice.fakeSessionArgument,
        ]).graph
        return (graph, try #require(graph.connections.core as? FakeMobileCore))
    }

    /// `graph` started, `split` connected and `work` attached.
    internal static func attachWork(_ graph: AppGraph) async throws {
        graph.start()
        try await until { graph.connections.links[work.machineId] == .connected }
        try await graph.connections.attach(machineId: work.machineId, sessionId: work.sessionId)
    }

    /// `view` in a window of the app's scene, shown; the caller hides the window when done.
    internal static func show(_ view: some View) throws -> (UIWindow, UIViewController) {
        let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        let host = UIHostingController(rootView: view)
        window.rootViewController = host
        window.makeKeyAndVisible()
        return (window, host)
    }

    /// Waits for `condition`, about 10 s on an idle simulator, failing the test when it doesn't come.
    internal static func until(_ condition: () -> Bool) async throws {
        var looked = 0
        while !condition(), looked < looks {
            looked += 1
            try await Task.sleep(for: .milliseconds(stepMilliseconds))
        }
        try #require(condition(), "timed out waiting")
    }

    /// The views of type `kind` under `view`, its own included.
    internal static func views<Kind: UIView>(_ kind: Kind.Type, in view: UIView) -> [Kind] {
        let own: [Kind] = (view as? Kind).map { [$0] } ?? []
        return own + view.subviews.flatMap { views(kind, in: $0) }
    }

    /// The titles the navigation bars under `view` show.
    internal static func titles(in view: UIView) -> [String] {
        views(UINavigationBar.self, in: view).flatMap { bar in
            (bar.items ?? []).compactMap(\.title)
        }
    }
}
