import NgtermCore
import NgtermCoreFakes
import NgtermFeatures
import Testing
import UIKit

@testable import NgTerm

/// The pick sheet over a session view in a window of the app's scene: it waits while an ask of the session is open,
/// shows once the ask is answered, and goes when the origin takes the pick back (answered elsewhere, its timeout ran
/// out), unanswered.
@MainActor
@Suite("the pick sheet over a session")
internal struct PickPromptTests {
    private static let pick = RemotePick(
        id: "switch",
        items: ["api", "web"].map { id in
            RemotePickItem(id: id, label: id, subtitle: nil, status: "blocked", statusColor: nil, statusShape: nil)
        },
        prompt: "Switch to",
        query: nil,
        allowCustom: false,
        selection: nil
    )

    @Test("a pick waits for the session's ask, shows after it, and goes unanswered when the origin takes it back")
    internal func waitsShowsAndGoes() async throws {
        let (graph, core) = try AppWindow.sessionWorld()
        let key = AppWindow.work
        try await AppWindow.attachWork(graph)
        let (window, host) = try AppWindow.show(AskHost(graph: graph, shown: AskHostShown(key)))
        defer { window.isHidden = true }

        core.emit(.ask(machineId: key.machineId, sessionId: key.sessionId, ask: FakeSessionWorld.deployAsk))
        core.emit(.pick(machineId: key.machineId, sessionId: key.sessionId, pick: Self.pick))
        try await AppWindow.until("the ask shows first") { host.presentedViewController != nil }
        try await AppWindow.until { graph.picks.find(key, pickId: Self.pick.id) != nil }

        graph.asks.answer(key, askId: FakeSessionWorld.deployAsk.id, reply: AskDraft.escaped)
        try await AppWindow.until("the ask's sheet goes") { host.presentedViewController == nil }
        try await AppWindow.until("the pick shows next") { host.presentedViewController != nil }
        #expect(graph.picks.find(key, pickId: Self.pick.id) != nil)

        core.emit(.pickDismiss(machineId: key.machineId, sessionId: key.sessionId, pickId: Self.pick.id))
        try await AppWindow.until("the pick's sheet goes") { host.presentedViewController == nil }
        try await Task.sleep(for: .milliseconds(500))
        #expect(core.calls.allSatisfy { $0.method != "answerPick" }, "taken back, it is not answered")
    }
}
