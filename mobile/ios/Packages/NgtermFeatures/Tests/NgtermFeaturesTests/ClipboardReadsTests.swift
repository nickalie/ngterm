import Foundation
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
    private static let copied = ClipboardPart(mime: "text/plain", data: Data("copied text".utf8))

    private let test = TestGraph()
    private let pane = PaneRef(machineId: "m1", sessionId: "s1", paneId: "P1")

    private var reads: ClipboardReads {
        test.graph.clipboardReads
    }

    private func request(
        _ id: UInt64,
        pane: PaneRef? = nil,
        listOnly: Bool = false,
        mimes: [String] = ["text/plain"],
        list: Bool = false,
        granted: Bool = false,
        canRemember: Bool = false
    ) async {
        let reading = pane ?? self.pane
        test.core.emit(
            .clipboardReadRequest(
                pane: reading,
                readId: id,
                clipboard: .standard,
                listOnly: listOnly,
                mimes: listOnly ? [] : mimes,
                list: listOnly || list,
                granted: granted,
                canRemember: canRemember
            )
        )
        await test.settled()
    }

    private func allowed(
        _ contents: [ClipboardPart] = [],
        available: [String] = [],
        remember: Bool = false
    ) -> ClipboardReply {
        .allow(contents: contents, available: available, remember: remember)
    }

    private func answers() -> [FakeCall] {
        test.core.calls.filter { $0.method == "answerClipboardRead" }
    }

    private func answer(_ id: UInt64, _ reply: ClipboardReply, pane: PaneRef? = nil) -> FakeCall {
        let reading = pane ?? self.pane
        return FakeCall("answerClipboardRead", [reading.machineId, reading.sessionId, reading.paneId, id, reply])
    }

    @Test("a read waits for the user, and Allow hands over what the preview showed")
    internal func allow() async {
        defer { test.remove() }
        test.clipboard.held = "copied text"
        await request(7)
        #expect(reads.pending == [ClipboardRead(pane: pane, readId: 7, listOnly: false)])
        #expect(answers().isEmpty)
        #expect(reads.preview(reads.pending[0]).contents == [Self.copied])
        test.clipboard.held = "copied later"
        reads.allow(reads.pending[0])
        #expect(answers() == [answer(7, allowed([Self.copied]))])
        #expect(reads.pending.isEmpty)
    }

    @Test("Deny refuses, and a clipboard with none of the types is allowed as nothing")
    internal func deny() async {
        defer { test.remove() }
        await request(1)
        await request(2)
        reads.deny(reads.pending[0])
        reads.allow(reads.pending[0])
        #expect(answers() == [answer(1, .deny), answer(2, allowed())])
    }

    @Test("each requested type the clipboard holds is handed over in order, with the listing")
    internal func types() async {
        defer { test.remove() }
        let png = ClipboardPart(mime: "image/png", data: Data([0x89, 0x50, 0x4E, 0x47, 0]))
        let html = ClipboardPart(mime: "text/html", data: Data("<b>hi</b>".utf8))
        test.clipboard.held = "copied text"
        test.clipboard.others = [html, png]
        await request(1, mimes: ["image/png", "application/x-missing", "text/plain"], list: true)
        reads.allow(reads.pending[0])
        let listed = ["text/plain", "text/html", "image/png"]
        #expect(answers() == [answer(1, allowed([png, Self.copied], available: listed))])
    }

    @Test("a listing or a read of no type is answered unasked with no data, sixteen types at most")
    internal func listing() async {
        defer { test.remove() }
        test.clipboard.others = (1...20).map { ClipboardPart(mime: "application/x-\($0)", data: Data([1])) }
        await request(1, listOnly: true)
        await request(2, mimes: [], list: true)
        #expect(reads.pending.isEmpty)
        let listed = (1...16).map { "application/x-\($0)" }
        #expect(answers() == [answer(1, allowed(available: listed)), answer(2, allowed(available: listed))])
    }

    @Test("a granted read is answered unasked, and remember is kept only where it can be")
    internal func grants() async {
        defer { test.remove() }
        test.clipboard.held = "copied text"
        await request(1, granted: true)
        #expect(reads.pending.isEmpty)
        await request(2, canRemember: true)
        await request(3)
        reads.allow(reads.pending[0], remember: true)
        reads.allow(reads.pending[0], remember: true)
        #expect(
            answers() == [
                answer(1, allowed([Self.copied])),
                answer(2, allowed([Self.copied], remember: true)),
                answer(3, allowed([Self.copied])),
            ]
        )
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
                answer(1, allowed([Self.copied])),
                answer(2, allowed([Self.copied]), pane: sibling),
                answer(4, allowed([Self.copied])),
            ]
        )
        #expect(reads.alwaysAllowed == ["m1"])
        try reads.revoke("m1")
        #expect(reads.alwaysAllowed.isEmpty)
        await request(5)
        #expect(reads.pending.count == 2)
    }

    @Test("an Ask again the store can't write still asks until the app ends")
    internal func revokeUnwritable() async throws {
        defer {
            try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: test.directory.path)
            test.remove()
        }
        await request(1)
        reads.allowAlways(reads.pending[0])
        #expect(ClipboardTrustStore(directory: test.directory).machines == ["m1"])
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: test.directory.path)
        #expect(throws: StoreError.self) {
            try reads.revoke("m1")
        }
        #expect(reads.alwaysAllowed.isEmpty)
        await request(2)
        #expect(reads.pending == [ClipboardRead(pane: pane, readId: 2, listOnly: false)])
        #expect(answers() == [answer(1, allowed())])
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
