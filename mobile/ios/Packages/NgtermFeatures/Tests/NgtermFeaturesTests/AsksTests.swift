import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// The open asks over the fake core (mobile/android's `AsksTest`): opened and restated by the core, dismissed by it,
/// answered once through it, let go with their session's detach, and taken back as the core suspends.
@MainActor
@Suite("the open asks")
internal struct AsksTests {
    private let test = TestGraph()
    private let key = SessionKey(machineId: "m1", sessionId: "s1")

    private var asks: Asks {
        test.graph.asks
    }

    // m1 connected, its sessions s1 and s2 listed and attached
    private func attach() async throws {
        try test.seed(TestGraph.machine(keyId: "k1"))
        test.core.sessions["m1"] = [fakeSession(id: "s1", name: "api"), fakeSession(id: "s2", name: "docs")]
        await test.connections.connect(TestGraph.machine(keyId: "k1").core).value
        await test.settled()
        try await test.connections.attach(machineId: "m1", sessionId: "s1")
        try await test.connections.attach(machineId: "m1", sessionId: "s2")
    }

    private func emit(_ events: MobileEvent...) async {
        for event in events {
            test.core.emit(event)
        }
        await test.settled()
    }

    private func answers() -> [FakeCall] {
        test.core.calls.filter { $0.method == "answerAsk" }
    }

    @Test("an ask opens, the same id again restates it in place of the first, and a dismissal closes it")
    internal func openAndDismiss() async throws {
        defer { test.remove() }
        try await attach()
        let ask = TestAsks.plain()
        var restated = ask
        restated.title = "Deploy to staging?"
        let other = SessionKey(machineId: "m1", sessionId: "s2")
        await emit(
            .ask(machineId: "m1", sessionId: "s1", ask: ask),
            .ask(machineId: "m1", sessionId: "s2", ask: ask),
            .ask(machineId: "m1", sessionId: "s1", ask: restated)
        )
        #expect(asks.open == [OpenAsk(key: other, ask: ask), OpenAsk(key: key, ask: restated)])
        await emit(.askDismiss(machineId: "m1", sessionId: "s1", askId: "A1"))
        #expect(asks.open == [OpenAsk(key: other, ask: ask)])
        #expect(asks.find(key, askId: "A1") == nil)
        #expect(asks.find(other, askId: "A1") != nil)
    }

    @Test("an answer goes through the core once, and one to an ask no longer open is left be")
    internal func answerOnce() async throws {
        defer { test.remove() }
        try await attach()
        await emit(.ask(machineId: "m1", sessionId: "s1", ask: TestAsks.plain()))
        let reply = AskReply(button: "yes", text: nil, selected: nil)
        #expect(asks.answer(key, askId: "A1", reply: reply))
        #expect(!asks.answer(key, askId: "A1", reply: reply))
        #expect(asks.open.isEmpty)
        #expect(answers() == [FakeCall("answerAsk", ["m1", "s1", "A1", reply])])
    }

    @Test("a detached session's asks go, and the core's suspend takes every ask back unanswered")
    internal func detachAndSuspend() async throws {
        defer { test.remove() }
        try await attach()
        await emit(
            .ask(machineId: "m1", sessionId: "s1", ask: TestAsks.plain()),
            .ask(machineId: "m1", sessionId: "s2", ask: TestAsks.plain())
        )
        test.connections.detach(machineId: "m1", sessionId: "s1")
        #expect(asks.open.map(\.key) == [SessionKey(machineId: "m1", sessionId: "s2")])
        test.graph.suspend()
        #expect(asks.open.isEmpty)
        #expect(answers().isEmpty)
        #expect(test.core.calls.last?.method == "suspend")
    }

    @Test("an ask the core tells after the suspend took them back opens nothing until the app resumes")
    internal func lateAskAfterSuspend() async throws {
        defer { test.remove() }
        try await attach()
        test.graph.suspend()
        await emit(.ask(machineId: "m1", sessionId: "s1", ask: TestAsks.plain()))
        #expect(asks.open.isEmpty)
        #expect(test.notices.posted.isEmpty, "and no notification goes up for it")
        test.graph.resume()
        #expect(test.core.calls.last?.method == "resume")
        await emit(.ask(machineId: "m1", sessionId: "s1", ask: TestAsks.plain()))
        #expect(asks.open.map(\.key) == [key], "handed over again after the reconnect, it opens")
    }

    @Test("a session shows while a view says so")
    internal func onScreen() {
        defer { test.remove() }
        asks.showing(key, isShown: true)
        #expect(asks.onScreen == [key])
        asks.showing(key, isShown: false)
        #expect(asks.onScreen.isEmpty)
    }
}
