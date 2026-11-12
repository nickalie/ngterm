import Foundation
import NgtermCore
import Testing

/// The facade's calls reaching the linked Rust core: what it answers about a machine it doesn't hold, the calls it
/// takes for nothing, and the suspend, resume and key change of a connect in flight (a silent listener holds one open).
@Suite("the live core's calls", .timeLimit(.minutes(1)))
internal struct LiveMobileCoreCallsTests {
    // a pane id the core reads as a pane's identity, of no session it holds
    private static let paneId = "6F9619FF-8B86-D011-B42D-00C04FC964FF"
    private static let cancel = AskReply(button: "Cancel", text: nil, selected: nil)

    private static func machine(port: UInt16, keyId: String = "k1") -> Machine {
        Machine(id: "m1", label: "box", host: "127.0.0.1", port: port, user: "me", keyId: keyId)
    }

    /// The core's first report of `machineId` being unreachable, skipping the states before it.
    private static func firstUnreachable(
        of machineId: String,
        in events: AsyncStream<MobileEvent>
    ) async -> String? {
        for await event in events {
            if case .machineState(machineId, .unreachable(let reason)) = event {
                return reason
            }
        }
        return nil
    }

    @Test("a session call about a machine that isn't connected is refused as not connected")
    internal func sessionCallsNeedTheMachine() async throws {
        let core = try LiveMobileCore(vault: StubVault(), hosts: MemoryHostKeys())
        await #expect(throws: MobileError.NotConnected(machineId: "m1")) {
            try await core.endSession(machineId: "m1", sessionId: "s1")
        }
        await #expect(throws: MobileError.NotConnected(machineId: "m1")) {
            try await core.attachSession(machineId: "m1", sessionId: "s1")
        }
        await #expect(throws: MobileError.NotConnected(machineId: "m1")) {
            _ = try await core.showPane(
                machineId: "m1",
                sessionId: "s1",
                paneId: Self.paneId,
                size: PaneSize(cols: 80, rows: 24, cellWidth: 10, cellHeight: 20)
            )
        }
    }

    @Test("a chat call about a machine that isn't connected is refused with the core's reason")
    internal func chatCallsNeedTheMachine() async throws {
        let core = try LiveMobileCore(vault: StubVault(), hosts: MemoryHostKeys())
        await #expect(throws: MobileError.NotConnected(machineId: "m1")) {
            try await core.openChat(machineId: "m1", sessionId: "s1", paneId: Self.paneId)
        }
        await #expect(throws: MobileError.Remote(reason: "the session has no such pane")) {
            try await core.openChat(machineId: "m1", sessionId: "s1", paneId: "not a pane")
        }
        await #expect(throws: MobileError.Remote(reason: "the pane's chat is not open")) {
            _ = try await core.loadOlder(machineId: "m1", sessionId: "s1", paneId: Self.paneId)
        }
        await #expect(throws: MobileError.NotConnected(machineId: "m1")) {
            _ = try await core.agentCommands(machineId: "m1", sessionId: "s1", paneId: Self.paneId)
        }
    }

    @Test("an upload to a machine that isn't connected is refused before a byte goes")
    internal func uploadNeedsTheMachine() async throws {
        let core = try LiveMobileCore(vault: StubVault(), hosts: MemoryHostKeys())
        let told = Counter()
        await #expect(throws: MobileError.NotConnected(machineId: "m1")) {
            _ = try await core.uploadFile(
                machineId: "m1",
                name: "notes.txt",
                bytes: Data("hello".utf8),
                progress: { _, _ in told.add() },
                cancel: UploadCancel()
            )
        }
        #expect(told.count == .zero)
    }

    @Test("the calls about what the core doesn't hold are taken, and the core goes on answering")
    internal func callsAboutNothingAreTaken() async throws {
        let core = try LiveMobileCore(vault: StubVault(), hosts: MemoryHostKeys())
        try core.useKey(machineId: "m1", keyId: "k2")
        try core.answerHostKey(machineId: "m1", fingerprint: "SHA256:abc", trust: true)
        try core.detachSession(machineId: "m1", sessionId: "s1")
        try core.networkChanged()
        try core.answerAsk(machineId: "m1", sessionId: "s1", askId: "a1", reply: Self.cancel)
        try core.hidePane(machineId: "m1", sessionId: "s1", paneId: Self.paneId)
        try core.takeLead(machineId: "m1", sessionId: "s1", paneId: Self.paneId)
        try core.answerClipboardRead(
            machineId: "m1",
            sessionId: "s1",
            paneId: Self.paneId,
            readId: 1,
            reply: .deny
        )
        try core.completeClipboardWrite(
            machineId: "m1",
            sessionId: "s1",
            paneId: Self.paneId,
            writeId: 1,
            status: .done
        )
        try core.closeChat(machineId: "m1", sessionId: "s1", paneId: Self.paneId)
        await #expect(throws: MobileError.NotConnected(machineId: "m1")) {
            _ = try await core.listSessions(machineId: "m1")
        }
    }

    @Test("a suspend cancels a connect in flight and closes its link, and a resume connects the machine again")
    internal func suspendAndResume() async throws {
        let listener = try #require(SilentListener())
        let key = try CoreKeys.generate(comment: "test")
        let core = try LiveMobileCore(vault: StubVault(keys: ["k1": key.privateKey]), hosts: MemoryHostKeys())
        let machine = Self.machine(port: listener.port)
        let connecting = Task {
            try await core.connect(machine: machine)
        }
        #expect(try await listener.waitForConnection(timeout: .seconds(10)))
        try core.suspend()
        await #expect(throws: MobileError.Cancelled) {
            try await connecting.value
        }
        #expect(try await listener.waitForClose(timeout: .seconds(10)))
        try core.resume()
        #expect(try await listener.waitForConnection(timeout: .seconds(10)))
        try core.disconnect(machineId: "m1")
        #expect(try await listener.waitForClose(timeout: .seconds(10)))
    }

    @Test("a key change reaches the machine's next attempt: the one a resume makes signs in with it")
    internal func useKeyReachesTheNextAttempt() async throws {
        let listener = try #require(SilentListener())
        let key = try CoreKeys.generate(comment: "test")
        let core = try LiveMobileCore(vault: StubVault(keys: ["k1": key.privateKey]), hosts: MemoryHostKeys())
        let machine = Self.machine(port: listener.port)
        let connecting = Task {
            try await core.connect(machine: machine)
        }
        #expect(try await listener.waitForConnection(timeout: .seconds(10)))
        try core.useKey(machineId: "m1", keyId: "k2")
        try core.suspend()
        await #expect(throws: MobileError.Cancelled) {
            try await connecting.value
        }
        try core.resume()
        // the vault holds no k2: the attempt fails on loading it, before it dials
        #expect(await Self.firstUnreachable(of: "m1", in: core.events) == "no key k2")
    }
}
