import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// The open picks over the fake core (mobile/android's `PickFlowTest`): opened and restated by the core, dismissed by
/// it, answered once through it, let go with their session's detach, and taken back as the core suspends; and what a
/// row answers.
@MainActor
@Suite("the open picks")
internal struct PicksTests {
    private let test = TestGraph()
    private let key = SessionKey(machineId: "m1", sessionId: "s1")

    private var picks: Picks {
        test.graph.picks
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
        test.core.calls.filter { $0.method == "answerPick" }
    }

    @Test("a pick opens, the same id again restates it in place of the first, and a dismissal closes it")
    internal func openAndDismiss() async throws {
        defer { test.remove() }
        try await attach()
        let pick = TestPicks.pick()
        var restated = pick
        restated.prompt = "Open"
        let other = SessionKey(machineId: "m1", sessionId: "s2")
        await emit(
            .pick(machineId: "m1", sessionId: "s1", pick: pick),
            .pick(machineId: "m1", sessionId: "s2", pick: pick),
            .pick(machineId: "m1", sessionId: "s1", pick: restated)
        )
        #expect(picks.open == [OpenPick(key: other, pick: pick), OpenPick(key: key, pick: restated)])
        await emit(.pickDismiss(machineId: "m1", sessionId: "s1", pickId: "K1"))
        #expect(picks.open == [OpenPick(key: other, pick: pick)])
        #expect(picks.find(key, pickId: "K1") == nil)
    }

    @Test("an answer goes through the core once, and one to a pick no longer open is left be")
    internal func answerOnce() async throws {
        defer { test.remove() }
        try await attach()
        await emit(.pick(machineId: "m1", sessionId: "s1", pick: TestPicks.pick()))
        #expect(picks.answer(key, pickId: "K1", reply: .picked(item: "web")))
        #expect(!picks.answer(key, pickId: "K1", reply: .cancelled))
        #expect(picks.open.isEmpty)
        #expect(answers() == [FakeCall("answerPick", ["m1", "s1", "K1", PickReply.picked(item: "web")])])
    }

    @Test("a detached session's picks go, the suspend takes every pick back, and none opens until the app resumes")
    internal func detachAndSuspend() async throws {
        defer { test.remove() }
        try await attach()
        await emit(
            .pick(machineId: "m1", sessionId: "s1", pick: TestPicks.pick()),
            .pick(machineId: "m1", sessionId: "s2", pick: TestPicks.pick())
        )
        test.connections.detach(machineId: "m1", sessionId: "s1")
        #expect(picks.open.map(\.key) == [SessionKey(machineId: "m1", sessionId: "s2")])
        test.graph.suspend()
        #expect(picks.open.isEmpty)
        await emit(.pick(machineId: "m1", sessionId: "s2", pick: TestPicks.pick()))
        #expect(picks.open.isEmpty)
        test.graph.resume()
        await emit(.pick(machineId: "m1", sessionId: "s2", pick: TestPicks.pick()))
        #expect(picks.open.count == 1, "handed over again after the reconnect, it opens")
        #expect(answers().isEmpty)
    }

    @Test("the rows are the linked core's, and the custom row answers with its text")
    internal func rows() {
        defer { test.remove() }
        let pick = TestPicks.pick(allowCustom: true)
        #expect(Picks.rows(of: pick, query: "") == [.item(index: 0), .item(index: 1), .item(index: 2)])
        #expect(Picks.rows(of: pick, query: "do") == [.item(index: 2)])
        let custom = Picks.rows(of: pick, query: "zzz")
        #expect(custom == [.custom(label: "Use \"zzz\"", query: "zzz")])
        #expect(PickChoice.reply(custom[0], items: pick.items) == .custom(query: "zzz"))
    }

    @Test("Return opens on the origin's selection when shown, else the first row, and a row answers with its item")
    internal func choice() {
        defer { test.remove() }
        let items = TestPicks.pick().items
        let rows: [PickRow] = [.item(index: 2), .item(index: 0)]
        #expect(PickChoice.initialRow(rows, items: items, selection: "api") == 1)
        #expect(PickChoice.initialRow(rows, items: items, selection: "web") == 0)
        #expect(PickChoice.initialRow(rows, items: items, selection: nil) == 0)
        #expect(PickChoice.reply(.item(index: 2), items: items) == .picked(item: "docs"))
        #expect(PickChoice.reply(.item(index: 9), items: items) == .cancelled)
    }
}
