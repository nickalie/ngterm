import Foundation
import NgtermCore
import NgtermCoreFakes
import Synchronization
import Testing

/// The fake core's chats (mobile/android's FakeMobileCoreTest on them): a chat opens only on a shown pane and closes
/// with it, its older pages come in turn, the agent's commands are the test's, and an upload stops at its cancel.
@Suite("the fake core's chats", .timeLimit(.minutes(1)))
internal struct FakeMobileCoreChatTests {
    private static let machine = Machine(id: "m1", label: "box", host: "box.example", port: 22, user: "me", keyId: "k1")
    private static let size = PaneSize(cols: 80, rows: 24, cellWidth: 10, cellHeight: 20)
    private static let pane = PaneRef(machineId: "m1", sessionId: "s1", paneId: "p1")

    // a core with box connected, s1 listed and its pane p1 shown
    private static func shown() async throws -> FakeMobileCore {
        let core = FakeMobileCore()
        core.sessions["m1"] = [fakeSession(id: "s1")]
        try await core.connect(machine: machine)
        _ = try await core.listSessions(machineId: "m1")
        _ = try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "p1", size: size)
        return core
    }

    @Test("a machine whose listing names agterm refuses the chat, its commands and uploads, as the core does")
    internal func agtermRefusesChat() async throws {
        let core = FakeMobileCore()
        core.sessions["m1"] = [fakeSession(id: "s1", peer: .agterm)]
        try await core.connect(machine: Self.machine)
        _ = try await core.listSessions(machineId: "m1")
        _ = try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "p1", size: Self.size)
        let refused = MobileError.Remote(reason: FakeMobileCore.noChatOnAgterm)
        await #expect(throws: refused) {
            try await core.openChat(machineId: "m1", sessionId: "s1", paneId: "p1")
        }
        await #expect(throws: refused) {
            _ = try await core.agentCommands(machineId: "m1", sessionId: "s1", paneId: "p1")
        }
        await #expect(throws: refused) {
            _ = try await core.uploadFile(
                machineId: "m1",
                name: "notes.txt",
                bytes: Data("hi".utf8),
                progress: { _, _ in
                    // nothing is sent
                },
                cancel: UploadCancel()
            )
        }
        #expect(core.openChats.isEmpty)
        #expect(core.uploads.isEmpty)
    }

    @Test("a chat opens only on a shown pane, and closes with its close or the pane's hide")
    internal func openAndClose() async throws {
        let core = try await Self.shown()
        await #expect(throws: MobileError.Remote(reason: FakeMobileCore.notShown)) {
            try await core.openChat(machineId: "m1", sessionId: "s1", paneId: "p2")
        }
        try await core.openChat(machineId: "m1", sessionId: "s1", paneId: "p1")
        #expect(core.openChats == [Self.pane])
        try core.closeChat(machineId: "m1", sessionId: "s1", paneId: "p1")
        #expect(core.openChats.isEmpty)
        try await core.openChat(machineId: "m1", sessionId: "s1", paneId: "p1")
        try core.hidePane(machineId: "m1", sessionId: "s1", paneId: "p1")
        #expect(core.openChats.isEmpty)
    }

    @Test("a chat opening delivers its pane's catch-up; a pane's chat action waits for its origin's reply")
    internal func catchUpAndReply() async throws {
        let core = try await Self.shown()
        let event = ChatEvent(id: "u1", cursor: 0, parent: nil, body: .user(text: "hi", images: 0))
        core.chatEvents[Self.pane] = [event]
        try await core.openChat(machineId: "m1", sessionId: "s1", paneId: "p1")
        #expect(core.emitted.last == .chat(pane: Self.pane, events: [event]))
        let handle = try await core.showPane(machineId: "m1", sessionId: "s1", paneId: "p1", size: Self.size)
        let fake = try #require(handle as? FakePane)
        fake.frames = { _ in Data() }
        let replied = Mutex<[FakeAction]>([])
        fake.onAction = { action in replied.withLock { $0.append(action) } }
        #expect(try await fake.sendPrompt(text: "go") == .sent)
        #expect(replied.withLock { $0 } == [.prompt("go")])
    }

    @Test("an open chat's older pages come in turn, then none; the agent's commands are the test's")
    internal func pagesAndCommands() async throws {
        let core = try await Self.shown()
        await #expect(throws: MobileError.Remote(reason: FakeMobileCore.notOpen)) {
            try await core.loadOlder(machineId: "m1", sessionId: "s1", paneId: "p1")
        }
        try await core.openChat(machineId: "m1", sessionId: "s1", paneId: "p1")
        let event = ChatEvent(id: "u1", cursor: 0, parent: nil, body: .user(text: "hi", images: 0))
        core.olderPages[Self.pane] = [[event]]
        #expect(try await core.loadOlder(machineId: "m1", sessionId: "s1", paneId: "p1") == [event])
        #expect(try await core.loadOlder(machineId: "m1", sessionId: "s1", paneId: "p1").isEmpty)
        await #expect(throws: MobileError.self) {
            try await core.agentCommands(machineId: "m1", sessionId: "s1", paneId: "p1")
        }
        let command = AgentCommand(name: "compact", description: "", kind: .builtin, argumentHint: nil)
        core.catalogs[Self.pane] = [command]
        #expect(try await core.agentCommands(machineId: "m1", sessionId: "s1", paneId: "p1") == [command])
    }

    @Test("an upload tells its progress and answers its stored path; one cancelled at its gate stores nothing")
    internal func upload() async throws {
        let core = try await Self.shown()
        let told = Mutex<[UInt64]>([])
        let path = try await core.uploadFile(
            machineId: "m1",
            name: "a.png",
            bytes: Data(count: 4),
            progress: { sent, _ in told.withLock { $0.append(sent) } },
            cancel: UploadCancel()
        )
        #expect(path == "/state/uploads/1/a.png")
        #expect(told.withLock { $0 } == [2, 4])
        #expect(core.uploads == [FakeUpload(machineId: "m1", name: "a.png", size: 4)])
        let gate = core.hold("uploadFile")
        let cancel = UploadCancel()
        let held = Task {
            try await core.uploadFile(
                machineId: "m1",
                name: "b.png",
                bytes: Data(count: 4),
                progress: { _, _ in
                    // what has gone is the first test's to read
                },
                cancel: cancel
            )
        }
        await gate.reached()
        cancel.cancel()
        await #expect(throws: MobileError.Cancelled) {
            try await held.value
        }
        #expect(core.uploads.count == 1)
    }
}
