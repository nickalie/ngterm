import NgtermFeatures
import SwiftUI
import Testing
import UIKit

@testable import NgTerm

/// The ask sheet over a session view in a window of the app's scene (a sheet needs one): a view that goes with its
/// sheet up (another session opened in its place, as a notification's tap does) leaves the ask open, unanswered.
@MainActor
@Suite("the ask sheet as its view goes")
internal struct AskPromptTeardownTests {
    // waits up to 5 s for `condition`
    private static func until(_ condition: () -> Bool) async throws {
        var waited = 0
        while !condition(), waited < 100 {
            waited += 1
            try await Task.sleep(for: .milliseconds(50))
        }
    }

    @Test("another session opened in place of the one asking leaves its ask open")
    internal func replaced() async throws {
        let graph = CoreChoice(arguments: [
            CoreChoice.fakeArgument, CoreChoice.fakeKeyArgument, CoreChoice.fakeSessionArgument,
            CoreChoice.fakeAskArgument,
        ]).graph
        graph.start()
        let key = SessionKey(machineId: FakeSessionWorld.machineId, sessionId: FakeSessionWorld.sessionId)
        try await Self.until { graph.connections.links[key.machineId] == .connected }
        try await graph.connections.attach(machineId: key.machineId, sessionId: key.sessionId)
        try await Self.until { graph.asks.find(key, askId: FakeSessionWorld.deployAsk.id) != nil }
        #expect(graph.asks.find(key, askId: FakeSessionWorld.deployAsk.id) != nil)

        let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        defer { window.isHidden = true }
        let shown = AskHostShown(key)
        let host = UIHostingController(rootView: AskHost(graph: graph, shown: shown))
        window.rootViewController = host
        window.makeKeyAndVisible()
        try await Self.until { host.presentedViewController != nil }
        #expect(host.presentedViewController != nil, "the ask shows in a sheet")

        shown.key = SessionKey(machineId: key.machineId, sessionId: "other")
        try await Self.until { host.presentedViewController == nil }
        #expect(host.presentedViewController == nil, "the sheet goes with its view")
        try await Task.sleep(for: .milliseconds(500))
        #expect(graph.asks.find(key, askId: FakeSessionWorld.deployAsk.id) != nil, "the ask stays open, unanswered")
    }
}
