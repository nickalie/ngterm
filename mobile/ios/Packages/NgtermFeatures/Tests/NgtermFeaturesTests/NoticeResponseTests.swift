import NgtermCore
import NgtermCoreFakes
import NgtermPlatform
import Testing

@testable import NgtermFeatures

/// What the notification action handler does with the user's response (mobile/android's `AskActionReceiver` tests):
/// a tap asks the app to open the session and pane; an ask's action answers through the core, with an input ask's typed
/// reply fitted to its field; an action on an ask no longer open takes its notice down and answers nothing.
@MainActor
@Suite("the notifications' responses")
internal struct NoticeResponseTests {
    private let test = TestGraph()
    private let key = SessionKey(machineId: "m1", sessionId: "s1")

    // m1 connected, s1 attached, and `ask` open on it, out of sight: its notice is up
    private func open(_ ask: RemoteAsk) async throws -> Notice {
        try test.seed(TestGraph.machine(keyId: "k1"))
        test.core.sessions["m1"] = [fakeSession(id: "s1", name: "api")]
        await test.connections.connect(TestGraph.machine(keyId: "k1").core).value
        await test.settled()
        try await test.connections.attach(machineId: "m1", sessionId: "s1")
        test.core.emit(.ask(machineId: "m1", sessionId: "s1", ask: ask))
        await test.settled()
        return try #require(test.notices.posted[Alert.askId(key, askId: ask.id)])
    }

    private func answers() -> [FakeCall] {
        test.core.calls.filter { $0.method == "answerAsk" }
    }

    @Test("a tap asks the app to open the session and its pane")
    internal func tap() async throws {
        defer { test.remove() }
        var ask = TestAsks.plain()
        ask.pane = "P2"
        let notice = try await open(ask)
        test.graph.alerts.respond(to: NoticeResponse(noticeId: notice.id, action: nil, info: notice.info))
        #expect(test.graph.openRequests.pending == OpenTarget(machineId: "m1", sessionId: "s1", paneId: "P2"))
        #expect(answers().isEmpty)
        test.graph.openRequests.consume(OpenTarget(machineId: "m1", sessionId: "s1", paneId: "P2"))
        #expect(test.graph.openRequests.pending == nil)
    }

    @Test("a button answers through the core, and the notice goes with the ask")
    internal func button() async throws {
        defer { test.remove() }
        let notice = try await open(TestAsks.plain())
        let action = try #require(notice.actions.first)
        test.graph.alerts.respond(to: NoticeResponse(noticeId: notice.id, action: action.id, info: notice.info))
        let reply = AskReply(button: "yes", text: nil, selected: nil)
        #expect(answers() == [FakeCall("answerAsk", ["m1", "s1", "A1", reply])])
        #expect(test.graph.asks.open.isEmpty)
        #expect(test.notices.posted.isEmpty)
    }

    @Test("a reply answers with what was typed, fitted to a line field")
    internal func reply() async throws {
        defer { test.remove() }
        let notice = try await open(TestAsks.asking(RemoteAskInput(multiline: false, value: "", placeholder: nil)))
        let response = NoticeResponse(noticeId: notice.id, action: "answer:yes", info: notice.info, text: "a\nb")
        test.graph.alerts.respond(to: response)
        let reply = AskReply(button: "yes", text: "a b", selected: nil)
        #expect(answers() == [FakeCall("answerAsk", ["m1", "s1", "A1", reply])])
    }

    @Test("an action on an ask no longer open takes its notice down and answers nothing")
    internal func stale() async throws {
        defer { test.remove() }
        let notice = try await open(TestAsks.plain())
        test.graph.suspend()
        // the system kept the notice the app withdrew (a process ended in the grace, say)
        test.notices.post(notice)
        test.graph.alerts.respond(to: NoticeResponse(noticeId: notice.id, action: "answer:yes", info: notice.info))
        #expect(answers().isEmpty)
        #expect(test.notices.posted.isEmpty)
    }
}
