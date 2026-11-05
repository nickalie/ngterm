import NgtermCore
import NgtermCoreFakes
import NgtermPlatform
import NgtermTerminal
import Testing

@testable import NgtermFeatures

/// A program's clipboard reads over the fake core (mobile/android's `ClipboardReadsTest`): asked, answered, remembered
/// per machine, dismissed by the core, and denied by the app when their session, machine, pane or the whole core goes.
@MainActor
@Suite("the clipboard reads")
internal struct ClipboardReadsTests {
    private let test = TestGraph()
    private let pane = PaneRef(machineId: "m1", sessionId: "s1", paneId: "P1")

    private var reads: ClipboardReads {
        test.graph.clipboardReads
    }

    private func request(_ id: UInt64, pane: PaneRef? = nil, listOnly: Bool = false) async {
        let reading = pane ?? self.pane
        test.core.emit(.clipboardReadRequest(pane: reading, readId: id, clipboard: .standard, listOnly: listOnly))
        await test.settled()
    }

    private func answers() -> [FakeCall] {
        test.core.calls.filter { $0.method == "answerClipboardRead" }
    }

    private func answer(_ id: UInt64, _ reply: ClipboardReply, pane: PaneRef? = nil) -> FakeCall {
        let reading = pane ?? self.pane
        return FakeCall("answerClipboardRead", [reading.machineId, reading.sessionId, reading.paneId, id, reply])
    }

    @Test("a read waits for the user, and Allow hands over the clipboard")
    internal func allow() async {
        defer { test.remove() }
        test.clipboard.held = "copied text"
        await request(7, listOnly: true)
        #expect(reads.pending == [ClipboardRead(pane: pane, readId: 7, listOnly: true)])
        #expect(answers().isEmpty)
        reads.allow(reads.pending[0])
        #expect(answers() == [answer(7, .allow(text: "copied text"))])
        #expect(reads.pending.isEmpty)
    }

    @Test("Deny gives an empty clipboard, and a clipboard with no text is allowed as none")
    internal func deny() async {
        defer { test.remove() }
        await request(1)
        await request(2)
        reads.deny(reads.pending[0])
        reads.allow(reads.pending[0])
        #expect(answers() == [answer(1, .deny), answer(2, .allow(text: nil))])
    }

    @Test("Always allow answers the machine's reads waiting and later ones unasked, kept across launches")
    internal func always() async throws {
        defer { test.remove() }
        test.clipboard.held = "copied text"
        let sibling = PaneRef(machineId: "m1", sessionId: "s2", paneId: "P2")
        let other = PaneRef(machineId: "m2", sessionId: "s9", paneId: "P9")
        await request(1)
        await request(2, pane: sibling)
        await request(3, pane: other)
        reads.allowAlways(reads.pending[0])
        #expect(reads.pending == [ClipboardRead(pane: other, readId: 3, listOnly: false)])
        #expect(ClipboardTrustStore(directory: test.directory).machines == ["m1"])
        await request(4)
        #expect(reads.pending.count == 1)
        #expect(
            answers() == [
                answer(1, .allow(text: "copied text")),
                answer(2, .allow(text: "copied text"), pane: sibling),
                answer(4, .allow(text: "copied text")),
            ]
        )
        #expect(reads.alwaysAllowed == ["m1"])
        try reads.revoke("m1")
        #expect(reads.alwaysAllowed.isEmpty)
        await request(5)
        #expect(reads.pending.count == 2)
    }

    @Test("a read the core dismisses leaves unanswered: the core denied it")
    internal func dismissed() async {
        defer { test.remove() }
        await request(1)
        await request(2)
        test.core.emit(.clipboardReadDismiss(pane: pane, readId: 1))
        await test.settled()
        #expect(reads.pending == [ClipboardRead(pane: pane, readId: 2, listOnly: false)])
        let elsewhere = PaneRef(machineId: "m1", sessionId: "s1", paneId: "other")
        test.core.emit(.clipboardReadDismiss(pane: elsewhere, readId: 2))
        await test.settled()
        #expect(reads.pending.count == 1)
        #expect(answers().isEmpty)
    }

    @Test("every read is answered: the app denies a detached session's, an ended pane's and a dropped machine's")
    internal func forcedDeny() async throws {
        defer { test.remove() }
        try test.seed(TestGraph.machine(keyId: "k1"))
        test.core.sessions["m1"] = [fakeSession(id: "s1"), fakeSession(id: "s2")]
        await test.connections.connect(TestGraph.machine(keyId: "k1").core).value
        await test.settled()
        try await test.connections.attach(machineId: "m1", sessionId: "s1")
        try await test.connections.attach(machineId: "m1", sessionId: "s2")
        let second = PaneRef(machineId: "m1", sessionId: "s2", paneId: "P2")
        await request(1)
        await request(2, pane: second)
        test.connections.detach(machineId: "m1", sessionId: "s1")
        #expect(answers() == [answer(1, .deny)])
        test.core.emit(.paneEnded(pane: second, end: .exited(code: 0, signal: nil)))
        await test.settled()
        #expect(answers() == [answer(1, .deny), answer(2, .deny, pane: second)])
        await request(3, pane: second)
        test.core.emit(.machineState(machineId: "m1", state: .reconnecting(nextInMs: 4_000)))
        await test.settled()
        #expect(answers().last == answer(3, .deny, pane: second))
        #expect(reads.pending.isEmpty)
    }

    @Test("the app's suspend denies every read waiting before the core closes the connections")
    internal func denyAll() async {
        defer { test.remove() }
        let other = PaneRef(machineId: "m2", sessionId: "s9", paneId: "P9")
        await request(1)
        await request(2, pane: other)
        test.graph.suspend()
        #expect(answers() == [answer(1, .deny), answer(2, .deny, pane: other)])
        #expect(reads.pending.isEmpty)
        let calls = test.core.calls.map(\.method).filter { $0 == "answerClipboardRead" || $0 == "suspend" }
        #expect(calls == ["answerClipboardRead", "answerClipboardRead", "suspend"])
    }

    @Test("a pinch keeps to the text sizes Settings offers")
    internal func textSizes() {
        let sizes = AppSettings.textSizes
        let pinched = TerminalPointer.textSizes
        #expect(Int(pinched.lowerBound) == sizes.lowerBound)
        #expect(Int(pinched.upperBound) == sizes.upperBound)
    }
}
