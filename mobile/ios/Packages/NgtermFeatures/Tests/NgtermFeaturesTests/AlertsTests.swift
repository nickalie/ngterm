import NgtermCore
import NgtermCoreFakes
import NgtermPlatform
import Testing

@testable import NgtermFeatures

/// What reaches the user as notifications (mobile/android's `AlertsTest`): the agents' notices and statuses, a machine
/// lost for good, and the asks out of sight, each on its session's thread, withdrawn as the rules say; the kinds the
/// user turned off are not posted.
@MainActor
@Suite("the alerts")
internal struct AlertsTests {
    private let test = TestGraph()
    private let key = SessionKey(machineId: "m1", sessionId: "s1")

    private var notices: RecordingNotices {
        test.notices
    }

    // m1 (build-01) connected, its session s1 (api) listed and attached
    private func attach() async throws {
        try test.seed(TestGraph.machine(keyId: "k1"))
        test.core.sessions["m1"] = [fakeSession(id: "s1", name: "api")]
        await test.connections.connect(TestGraph.machine(keyId: "k1").core).value
        await test.settled()
        try await test.connections.attach(machineId: "m1", sessionId: "s1")
    }

    private func emit(_ events: MobileEvent...) async {
        for event in events {
            test.core.emit(event)
        }
        await test.settled()
    }

    private func status(_ status: String, changed: Bool) -> MobileEvent {
        let said = RemoteStatus(status: status, blink: false, color: nil, shape: nil, pane: "P2", changedAt: nil)
        return .sessionStatus(machineId: "m1", sessionId: "s1", status: said, changed: changed)
    }

    @Test("an agent's notice posts on its session's thread, its tap leading to the session and pane")
    internal func notice() async throws {
        defer { test.remove() }
        try await attach()
        let sent = RemoteNotice(title: "Build done", body: "All green", pane: "P2", source: "agent")
        await emit(.notify(machineId: "m1", sessionId: "s1", notice: sent))
        let notice = try #require(notices.posted.values.first)
        #expect(notice.title == "Build done")
        #expect(notice.subtitle == "api")
        #expect(notice.body == "All green")
        #expect(notice.threadId == "session:m1:s1")
        #expect(notice.info == ["machine": "m1", "session": "s1", "pane": "P2"])
        #expect(notice.actions.isEmpty)
        test.connections.detach(machineId: "m1", sessionId: "s1")
        #expect(notices.posted.isEmpty, "a detached session's notices go")
    }

    @Test("a session renamed or given a new context on its stream is posted so")
    internal func streamSaid() async throws {
        defer { test.remove() }
        try await attach()
        let sent = RemoteNotice(title: "", body: "No title", pane: nil, source: "notify")
        await emit(
            .sessionName(machineId: "m1", sessionId: "s1", name: "Fix Linux"),
            .sessionContext(machineId: "m1", sessionId: "s1", context: "Sidebar rename drops the suffix"),
            status("blocked", changed: true),
            .notify(machineId: "m1", sessionId: "s1", notice: sent)
        )
        let waiting = try #require(notices.posted[Alert.statusId(key)])
        #expect(waiting.title == "Fix Linux is waiting for you")
        #expect(waiting.body == "Sidebar rename drops the suffix")
        let notice = try #require(notices.posted.values.first { $0.id.hasPrefix("notice:") })
        #expect(notice.title == "Fix Linux")
        // a cleared context is a clear, not a gap the listing fills
        await emit(
            .sessionContext(machineId: "m1", sessionId: "s1", context: nil),
            status("completed", changed: true)
        )
        let done = try #require(notices.posted[Alert.statusId(key)])
        #expect(done.body == "On build-01")
    }

    @Test("in the foreground an agent's notice shows beside its session, a status or an ask of a session on screen not")
    internal func foreground() async throws {
        defer { test.remove() }
        try await attach()
        let sent = RemoteNotice(title: "Build done", body: "All green", pane: "P2", source: "agent")
        await emit(.notify(machineId: "m1", sessionId: "s1", notice: sent), status("blocked", changed: true))
        let notice = try #require(notices.posted.values.first { $0.id.hasPrefix("notice:") })
        let status = try #require(notices.posted[Alert.statusId(key)])
        // as the app's delegate reads a notice it is about to show: its id, and its session from its info
        let shows = { (notice: Notice, onScreen: Set<SessionKey>) -> Bool in
            let about = notice.info["machine"].flatMap { machineId in
                notice.info["session"].map { SessionKey(machineId: machineId, sessionId: $0) }
            }
            return Alert.showsInForeground(id: notice.id, key: about, onScreen: onScreen)
        }
        #expect(shows(notice, [key]), "an agent's notice, as mobile/android's, beside its session too")
        #expect(!shows(status, [key]), "the session's view says its status")
        #expect(shows(status, [SessionKey(machineId: "m1", sessionId: "s2")]), "another session's view doesn't")
        #expect(!Alert.showsInForeground(id: Alert.askId(key, askId: "A1"), key: key, onScreen: [key]))
        #expect(Alert.showsInForeground(id: Alert.lostId("m1"), key: nil, onScreen: [key]))
    }

    @Test("a change to waiting or done posts once per session, a snapshot's status doesn't, anything else withdraws")
    internal func statuses() async throws {
        defer { test.remove() }
        try await attach()
        await emit(status("blocked", changed: false))
        #expect(notices.posted.isEmpty)
        await emit(status("blocked", changed: true))
        let waiting = try #require(notices.posted[Alert.statusId(key)])
        #expect(waiting.title == "api is waiting for you")
        #expect(waiting.body == "On build-01")
        await emit(status("completed", changed: true))
        #expect(notices.posted[Alert.statusId(key)]?.title == "api finished")
        await emit(status("active", changed: true))
        #expect(notices.posted.isEmpty)
        test.graph.asks.showing(key, isShown: true)
        await emit(status("blocked", changed: true))
        #expect(notices.posted.isEmpty, "a session on screen says it itself")
    }

    @Test("the kinds turned off are not posted")
    internal func kindsOff() async throws {
        defer { test.remove() }
        try await attach()
        try test.graph.notificationPrefs.set(.waiting, isOn: false)
        try test.graph.notificationPrefs.set(.notices, isOn: false)
        await emit(
            status("blocked", changed: true),
            .notify(machineId: "m1", sessionId: "s1", notice: RemoteNotice(title: "", body: "x", pane: nil, source: ""))
        )
        #expect(notices.posted.isEmpty)
        await emit(status("completed", changed: true))
        #expect(notices.upIds == [Alert.statusId(key)])
    }

    @Test("a machine with sessions attached lost for good posts, a changed host key says so, a connect withdraws it")
    internal func lost() async throws {
        defer { test.remove() }
        try await attach()
        await emit(.machineState(machineId: "m1", state: .unreachable(reason: "no route to host")))
        let lost = try #require(notices.posted[Alert.lostId("m1")])
        #expect(lost.title == "Lost build-01")
        #expect(lost.body == "no route to host")
        #expect(lost.threadId == "machine:m1")
        await emit(.machineState(machineId: "m1", state: .connected))
        #expect(notices.posted.isEmpty)
        await emit(
            .hostKeyChanged(machineId: "m1", pinned: "SHA256:a", offered: "SHA256:b"),
            .machineState(machineId: "m1", state: .unreachable(reason: FakeMobileCore.hostKeyChanged))
        )
        #expect(notices.posted[Alert.lostId("m1")]?.body.contains("host key changed") == true)
    }

    @Test("an ask out of sight posts with its buttons as actions, a reply, or a tap into the app")
    internal func askDeliveries() async throws {
        defer { test.remove() }
        try await attach()
        await emit(.ask(machineId: "m1", sessionId: "s1", ask: TestAsks.plain()))
        let plain = try #require(notices.posted[Alert.askId(key, askId: "A1")])
        #expect(plain.title == "Deploy to production?")
        #expect(plain.subtitle == "api on build-01")
        #expect(plain.info["ask"] == "A1")
        #expect(plain.actions.map(\.id) == ["answer:yes", "answer:no"])
        #expect(plain.actions.allSatisfy { $0.replyPlaceholder == nil })

        var input = TestAsks.asking(RemoteAskInput(multiline: false, value: "", placeholder: "Branch"))
        input.id = "A2"
        await emit(.ask(machineId: "m1", sessionId: "s1", ask: input))
        let reply = try #require(notices.posted[Alert.askId(key, askId: "A2")])
        #expect(reply.actions.map(\.replyPlaceholder) == ["Branch", nil])

        var list = TestAsks.choosing(TestAsks.choices(multiple: false, checked: [], other: nil))
        list.id = "A3"
        await emit(.ask(machineId: "m1", sessionId: "s1", ask: list))
        let choice = try #require(notices.posted[Alert.askId(key, askId: "A3")])
        #expect(choice.actions.isEmpty)
        #expect(choice.body == "Tap to answer in ngterm.")
    }

    @Test("an ask is withdrawn on screen, answered, dismissed, and when the core suspends")
    internal func askWithdrawn() async throws {
        defer { test.remove() }
        try await attach()
        let id = Alert.askId(key, askId: "A1")
        await emit(.ask(machineId: "m1", sessionId: "s1", ask: TestAsks.plain()))
        test.graph.asks.showing(key, isShown: true)
        #expect(notices.posted[id] == nil, "on screen it is a sheet")
        test.graph.asks.showing(key, isShown: false)
        #expect(notices.posted[id] != nil)
        await emit(.askDismiss(machineId: "m1", sessionId: "s1", askId: "A1"))
        #expect(notices.posted[id] == nil)
        await emit(.ask(machineId: "m1", sessionId: "s1", ask: TestAsks.plain()))
        test.graph.suspend()
        #expect(notices.posted.isEmpty, "the origin took it back")
        #expect(notices.log.suffix(2) == ["+" + id, "-" + id])
    }
}
