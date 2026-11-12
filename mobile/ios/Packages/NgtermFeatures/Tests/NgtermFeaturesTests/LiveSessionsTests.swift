import Foundation
import NgtermCore
import NgtermCoreFakes
import NgtermPlatform
import Testing

@testable import NgtermFeatures

/// The attached sessions' live state over the fake core, as mobile/android's `LiveSessionsTest` has it, and the
/// attached set kept for a background refresh.
@MainActor
@Suite("the live sessions")
internal struct LiveSessionsTests {
    private let test = TestGraph()
    private let key = SessionKey(machineId: "m1", sessionId: "s1")
    private let pane = PaneRef(machineId: "m1", sessionId: "s1", paneId: "P1")

    private var live: LiveSessions {
        test.graph.live
    }

    // m1 connected and listed with `sessions`, and those of them in `attach` attached
    private func attach(_ attach: [String] = ["s1"], of sessions: [String] = ["s1"]) async throws {
        try test.seed(TestGraph.machine(keyId: "k1"))
        test.core.sessions["m1"] = sessions.map { fakeSession(id: $0) }
        await test.connections.connect(TestGraph.machine(keyId: "k1").core).value
        await test.settled()
        for sessionId in attach {
            try await test.connections.attach(machineId: "m1", sessionId: sessionId)
        }
    }

    private func emit(_ events: MobileEvent...) async {
        for event in events {
            test.core.emit(event)
        }
        await test.settled()
    }

    private func lists() -> Int {
        test.core.calls.count { $0.method == "listSessions" }
    }

    @Test("a session's events land in its state, and a cleared HUD goes")
    internal func sessionEvents() async throws {
        defer { test.remove() }
        try await attach()
        let layout = RemoteLayout(panes: ["P1", "P2"], primary: "P1", axis: "vertical", shown: true)
        let status = RemoteStatus(
            status: "active",
            blink: false,
            color: "#5fbfaa",
            shape: "circle",
            pane: nil,
            changedAt: nil
        )
        let hud = RemoteHud(
            message: "Building",
            detail: "step 2",
            spinner: "bar",
            backgroundColor: nil,
            textColor: nil,
            sizePercent: nil,
            position: "top",
            markdown: false,
            fontSize: nil,
            sticky: false,
            frame: true,
            pane: nil,
            generation: 1,
            remaining: nil
        )
        await emit(
            .sessionConnection(machineId: "m1", sessionId: "s1", state: .connected),
            .sessionLayout(machineId: "m1", sessionId: "s1", layout: layout),
            .sessionStatus(machineId: "m1", sessionId: "s1", status: status, changed: true),
            .sessionContext(machineId: "m1", sessionId: "s1", context: "Rewrite the guide"),
            .hud(machineId: "m1", sessionId: "s1", hud: hud)
        )
        let expected = LiveSession(
            connection: .connected,
            status: status,
            context: "Rewrite the guide",
            layout: layout,
            hud: hud,
            isStatusSaid: true,
            isContextSaid: true
        )
        #expect(live.sessions[key] == expected)
        await emit(.hud(machineId: "m1", sessionId: "s1", hud: nil))
        #expect(live.sessions[key]?.hud == nil)
    }

    @Test("a stream failing before its session showed lists the machine again, and a session gone there leaves")
    internal func failedStream() async throws {
        defer { test.remove() }
        try await attach()
        let before = lists()
        // ended elsewhere after the listing the attach went by
        test.core.sessions["m1"] = []
        await emit(.sessionConnection(machineId: "m1", sessionId: "s1", state: .failed(reason: "no such session")))
        await Wait.until { live.sessions[key] == nil }
        #expect(lists() == before + 1)
        #expect(test.connections.attached["m1"] == nil)
        // a stream that has shown its session failing is retried by the core, not listed again
        test.core.sessions["m1"] = [fakeSession(id: "s1")]
        await test.connections.listSessions(machineId: "m1")
        try await test.connections.attach(machineId: "m1", sessionId: "s1")
        let layout = RemoteLayout(panes: ["P1"], primary: "P1", axis: nil, shown: false)
        await emit(.sessionLayout(machineId: "m1", sessionId: "s1", layout: layout))
        let shown = lists()
        await emit(.sessionConnection(machineId: "m1", sessionId: "s1", state: .failed(reason: "link down")))
        #expect(lists() == shown)
        #expect(live.sessions[key]?.connection == .failed(reason: "link down"))
    }

    @Test("a stream failing again before its session showed lists the machine once per run of failures")
    internal func failingAgain() async throws {
        defer { test.remove() }
        try await attach()
        let before = lists()
        await emit(
            .sessionConnection(machineId: "m1", sessionId: "s1", state: .failed(reason: "busy")),
            .sessionConnection(machineId: "m1", sessionId: "s1", state: .failed(reason: "busy"))
        )
        await Wait.until { lists() == before + 1 }
        await emit(
            .sessionConnection(machineId: "m1", sessionId: "s1", state: .connecting),
            .sessionConnection(machineId: "m1", sessionId: "s1", state: .failed(reason: "busy"))
        )
        await Wait.until { lists() == before + 2 }
    }

    @Test("a pane's events land in its state, and a fresh terminal clears its end")
    internal func paneEvents() async throws {
        defer { test.remove() }
        try await attach()
        await emit(.paneLead(pane: pane, role: .follower, covered: true), .paneTitle(pane: pane, title: "vim"))
        #expect(live.sessions[key]?.panes["P1"] == PaneLive(role: .follower, isCovered: true, title: "vim"))
        await emit(.paneEnded(pane: pane, end: .lost(reason: "link down")))
        let ended = PaneLive(role: .follower, isCovered: false, end: .lost(reason: "link down"), title: "vim")
        #expect(live.sessions[key]?.panes["P1"] == ended)
        await emit(.paneAttached(pane: pane))
        #expect(live.sessions[key]?.panes["P1"]?.end == nil)
    }

    @Test("a routed pane gets its frames, and an unrouted one none")
    internal func routing() async {
        defer { test.remove() }
        let frames = TestFrames()
        live.route(pane, to: frames)
        await emit(
            .paneDirty(pane: pane),
            .paneAttached(pane: pane),
            .paneDirty(pane: PaneRef(machineId: "m1", sessionId: "s1", paneId: "P2"))
        )
        live.route(pane, to: nil)
        await emit(.paneDirty(pane: pane))
        #expect(frames.seen == ["dirty", "attached"])
    }

    @Test("a program's write goes to the device's clipboard with every part, and its status back to the core")
    internal func clipboardWrite() async {
        defer { test.remove() }
        let image = ClipboardPart(mime: "image/png", data: Data([1, 2, 3]))
        let text = ClipboardPart(mime: "text/plain", data: Data("copied".utf8))
        await emit(
            .clipboardWrite(pane: pane, writeId: 1, contents: [text, image]),
            .clipboardWrite(pane: pane, writeId: 2, contents: [])
        )
        #expect(test.clipboard.partWrites == [[text, image], []])
        let completed = test.core.calls.filter { $0.method == "completeClipboardWrite" }
        #expect(
            completed == [
                FakeCall("completeClipboardWrite", ["m1", "s1", pane.paneId, UInt64(1), ClipboardWriteStatus.done]),
                FakeCall("completeClipboardWrite", ["m1", "s1", pane.paneId, UInt64(2), ClipboardWriteStatus.done]),
            ]
        )
    }

    @Test("a detached session's state goes")
    internal func detached() async throws {
        defer { test.remove() }
        try await attach()
        await emit(.sessionContext(machineId: "m1", sessionId: "s1", context: "notes"))
        #expect(live.sessions[key]?.context == "notes")
        test.connections.detach(machineId: "m1", sessionId: "s1")
        #expect(live.sessions[key] == nil)
    }

    @Test("the attached set is kept as it changes: attached, detached, ended, and its machine disconnected")
    internal func persisted() async throws {
        defer { test.remove() }
        try await attach(["s1", "s2"], of: ["s1", "s2", "s3"])
        let stored = { AttachedStore(directory: test.directory).attached }
        #expect(stored() == ["m1": ["s1", "s2"]])
        test.connections.detach(machineId: "m1", sessionId: "s1")
        #expect(stored() == ["m1": ["s2"]])
        try await test.connections.attach(machineId: "m1", sessionId: "s3")
        try await test.connections.endSession(machineId: "m1", sessionId: "s2")
        #expect(stored() == ["m1": ["s3"]])
        test.connections.disconnect(machineId: "m1")
        #expect(stored().isEmpty)
    }
}
