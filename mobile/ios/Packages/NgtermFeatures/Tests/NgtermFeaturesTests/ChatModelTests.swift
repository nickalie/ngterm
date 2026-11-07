import Foundation
import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// A pane's chat over the fake core (mobile/android's `ChatViewModelTest`): the prompt and Stop, the cards and their
/// answers, what an action that did nothing says, and older pages.
@MainActor
@Suite("a pane's chat")
internal struct ChatModelTests {
    private let world = ChatWorld()

    // the chat open, its conversation named and its backlog read
    private func live() async throws -> (ChatModel, FakePane) {
        try await world.open()
        await world.chat(ChatSamples.event("conv", ChatSamples.conversation), ChatSamples.event("live", .live))
        return (try #require(world.chat), try #require(world.fakePane))
    }

    @Test("Send goes to the agent as a prompt, its ends trimmed, and the field lets it go")
    internal func send() async throws {
        defer { world.remove() }
        let (chat, pane) = try await live()
        chat.composer.edit("  Fix the guide \n")
        chat.send()
        await Wait.until { !chat.snapshot.isBusy }
        #expect(pane.actions == [.prompt("Fix the guide")])
        #expect(chat.composer.text.isEmpty)
        #expect(chat.snapshot.problem == nil)
    }

    @Test("an action that sent nothing says why and keeps the prompt; one at a time")
    internal func problems() async throws {
        defer { world.remove() }
        let (chat, pane) = try await live()
        pane.outcome = .dialogOpen
        let gate = FakeGate<Void>()
        pane.gate = gate
        chat.composer.edit("Fix it")
        chat.send()
        chat.send()
        #expect(chat.snapshot.isBusy)
        gate.open()
        await Wait.until { !chat.snapshot.isBusy }
        #expect(pane.actions.count == 1, "a second Send while one is on its way sends nothing")
        #expect(chat.snapshot.problem == .dialogOpen)
        #expect(chat.composer.text == "Fix it")
        pane.actionError = .Refused(reason: "the pane runs no agent the chat can answer")
        chat.send()
        await Wait.until { !chat.snapshot.isBusy }
        #expect(chat.snapshot.problem == .refused(reason: "the pane runs no agent the chat can answer"))
        chat.problemShown()
        #expect(chat.snapshot.problem == nil)
    }

    @Test("an approval comes up as a card with the dialog's labels, and goes once answered here")
    internal func approval() async throws {
        defer { world.remove() }
        let (chat, pane) = try await live()
        pane.labels = ["Yes", "Yes, and don't ask again", "No"]
        await world.chat(ChatSamples.event("ap1", .approval(approval: ChatSamples.approval)))
        await Wait.until { chat.snapshot.approval?.labels.count == 3 }
        #expect(chat.snapshot.awaitsCard)
        chat.composer.edit("not now")
        chat.send()
        #expect(pane.actions.isEmpty, "a prompt now would go to its dialog")
        // a card another one replaced sends nothing
        chat.answerApproval(id: "ap0", label: "Yes")
        #expect(chat.snapshot.problem == .notOnScreen)
        pane.outcome = .notOnScreen
        chat.answerApproval(id: "ap1", label: "Yes")
        await Wait.until { !chat.snapshot.isBusy }
        #expect(chat.snapshot.approval != nil, "a dialog the core can't confirm leaves the card up")
        pane.outcome = .sent
        chat.answerApproval(id: "ap1", label: "Yes")
        await Wait.until { !chat.snapshot.isBusy }
        #expect(chat.snapshot.approval == nil)
        #expect(pane.actions.last == .approval(ChatSamples.approval, label: "Yes"))
    }

    @Test("a question still running is a card; answered, or its picker gone, it goes")
    internal func question() async throws {
        defer { world.remove() }
        let (chat, pane) = try await live()
        await world.chat(ChatSamples.event("q1", ChatSamples.question()))
        #expect(chat.snapshot.question?.id == "q1")
        pane.outcome = .notOnScreen
        chat.answerQuestion(id: "q1", answers: [QuestionAnswer(picked: ["fish"], other: nil)])
        await Wait.until { !chat.snapshot.isBusy }
        #expect(chat.snapshot.question == nil, "a picker no longer on screen was answered elsewhere")
        await world.chat(ChatSamples.event("q1", ChatSamples.question(.ok)))
        #expect(chat.snapshot.question == nil)
    }

    @Test("Stop interrupts a turn the agent works on, and not one interrupted already")
    internal func stop() async throws {
        defer { world.remove() }
        let (chat, pane) = try await live()
        #expect(!chat.isRunning)
        let working = RemoteStatus(status: "active", blink: false, color: nil, shape: nil, pane: "P1", changedAt: nil)
        await world.world.emit(.sessionStatus(machineId: "m1", sessionId: "s1", status: working, changed: true))
        #expect(chat.isRunning)
        chat.stop()
        await Wait.until { !chat.snapshot.isBusy }
        #expect(pane.actions == [.interrupt])
        await world.chat(ChatSamples.event("n1", .notice(kind: .interrupt, text: "Interrupted", detail: nil)))
        #expect(!chat.isRunning, "the status stays working, but the turn was interrupted")
        chat.stop()
        #expect(pane.actions == [.interrupt])
        // a prompt typed starts a turn again
        await world.chat(ChatSamples.event("u2", ChatSamples.user("go on")))
        #expect(chat.isRunning)
    }

    @Test("an older page goes in above, keeping the row that was on top, until the start")
    internal func olderPages() async throws {
        defer { world.remove() }
        let (chat, _) = try await live()
        await world.chat(ChatSamples.event("u5", ChatSamples.user("five")))
        world.core.olderPages[ChatWorld.pane] = [[ChatSamples.event("u4", ChatSamples.user("four"))], []]
        chat.loadOlder()
        await Wait.until { chat.snapshot.older == .more && chat.snapshot.items.count == 2 }
        #expect(chat.snapshot.items.map(\.key) == ["e:u4", "e:u5"])
        #expect(chat.snapshot.anchor == "e:u5")
        chat.loadOlder()
        await Wait.until { chat.snapshot.older == .atStart }
        world.core.failNext("loadOlder", with: .Remote(reason: "the origin gave no answer"))
        chat.loadOlder()
        #expect(chat.snapshot.older == .atStart, "nothing comes before the start")
    }

    @Test("a reset starts a new conversation under a divider, the account kept")
    internal func reset() async throws {
        defer { world.remove() }
        let (chat, _) = try await live()
        let usage = ChatUsage(usedPercent: 72, resetsAt: nil)
        await world.chat(
            ChatSamples.event("acct", .account(email: "dev@example.com", weekly: usage)),
            ChatSamples.event("u1", ChatSamples.user("old"))
        )
        await world.chat(ChatSamples.event("reset", .reset), ChatSamples.event("v1", ChatSamples.user("new")))
        #expect(chat.snapshot.items.map(\.key) == ["divider", "e:v1"])
        #expect(chat.snapshot.account?.email == "dev@example.com")
    }

    @Test("an attached file goes to the machine and its path into the prompt; Cancel stops it while bytes are to go")
    internal func upload() async throws {
        defer { world.remove() }
        let (chat, _) = try await live()
        chat.composer.edit("Look at")
        chat.attach(name: "shot.png", size: 4) { Data([1, 2, 3, 4]) }
        await Wait.until { chat.snapshot.upload == nil && chat.composer.text.contains("shot.png") }
        #expect(chat.composer.text == "Look at /state/uploads/1/shot.png ")
        #expect(world.core.uploads == [FakeUpload(machineId: "m1", name: "shot.png", size: 4)])
        // held halfway: Cancel stops it, and nothing is stored
        let gate = world.core.hold("uploadFile")
        chat.attach(name: "big.bin", size: 8) { Data(count: 8) }
        await gate.reached()
        await Wait.until { chat.snapshot.upload == .sending(name: "big.bin", sent: 4, total: 8, isSaving: false) }
        chat.cancelUpload()
        #expect(chat.snapshot.upload == nil)
        // the cancel reaches the core through the upload's token, which a Swift task's cancel alone would not
        #expect(world.core.uploadTokens.last?.isCancelled == true)
        gate.open()
        // a later upload that runs to its end: the cancelled one, ahead of it, never stored its file
        chat.attach(name: "small.bin", size: 2) { Data(count: 2) }
        await Wait.until { world.core.uploads.count == 2 }
        #expect(world.core.uploads.map(\.name) == ["shot.png", "small.bin"], "the cancelled upload stored nothing")
        chat.composer.edit("")
        // over the cap: refused before a byte is read
        chat.attach(name: "huge.bin", size: ChatUpload.cap + 1) { Data() }
        #expect(chat.snapshot.upload == .failed(name: "huge.bin", problem: .tooLarge))
        chat.uploadShown()
        #expect(chat.snapshot.upload == nil)
    }
}
