import Foundation
import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgTerm

/// The scripted Claude Code the chat's UI tests talk to: what it tells the app for each prompt and answer, at once.
@MainActor
@Suite("the fake chat's agent")
internal struct FakeChatWorldTests {
    private static let instant = FakeChatWorld.Pace(typing: .zero, slowTyping: .zero)

    private let core = FakeMobileCore()

    // the agent's pane, its chat's events told through `core`
    private func agentPane() -> FakePane {
        FakeChatWorld(core: core, pace: Self.instant) { _ in
            // every cue comes at once
        }
        .pane { _ in Data() }
    }

    // the chat events told so far, oldest first
    private func told() -> [ChatEvent] {
        core.emitted.flatMap { event -> [ChatEvent] in
            guard case .chat(let pane, let events) = event, pane == FakeChatWorld.pane else {
                return []
            }
            return events
        }
    }

    private func bodies() -> [ChatBody] {
        told().map(\.body)
    }

    // the latest permission prompt told, by kind: what the agent tells after it may have come already
    private func lastPrompt() throws -> (id: String, approval: ChatApproval) {
        let prompts = told().compactMap { event -> (id: String, approval: ChatApproval)? in
            guard case .approval(let approval) = event.body else {
                return nil
            }
            return (event.id, approval)
        }
        return try #require(prompts.last, "no permission prompt in \(bodies())")
    }

    @Test("the session world with its agent announces it, opens its chat on the catch-up, and answers on its pane")
    internal func seeded() throws {
        let graph = CoreChoice(arguments: [
            CoreChoice.fakeArgument, CoreChoice.fakeKeyArgument, CoreChoice.fakeSessionArgument,
            CoreChoice.fakeChatArgument,
        ]).graph
        let seededCore = try #require(graph.connections.core as? FakeMobileCore)
        #expect(seededCore.attachEvents[FakeSessionWorld.sessionId]?.last == FakeChatWorld.agentEvent)
        #expect(seededCore.chatEvents[FakeChatWorld.pane] == FakeChatWorld.catchUp)
        let size = PaneSize(cols: 0, rows: 0, cellWidth: 0, cellHeight: 0)
        let agent = seededCore.paneFactory(FakeSessionWorld.machineId, FakeSessionWorld.sessionId, "P1", size)
        let other = seededCore.paneFactory(FakeSessionWorld.machineId, FakeSessionWorld.sessionId, "P2", size)
        #expect((agent as? FakePane)?.labels == ["Yes", "No"])
        #expect((other as? FakePane)?.labels.isEmpty == true)
    }

    @Test("a prompt shows as the user's row, then its reply")
    internal func prompt() async throws {
        let pane = agentPane()
        #expect(try await pane.sendPrompt(text: "Fix the guide") == .sent)
        #expect(bodies() == [.user(text: "Fix the guide", images: 0), .assistant(text: "Done: Fix the guide.")])
        _ = try await pane.sendPrompt(text: "Check it, take your time")
        #expect(bodies().last == .assistant(text: "Done: Check it, take your time."))
        _ = try await pane.sendPrompt(text: "write a lot")
        guard case .assistant(let text) = bodies().last else {
            Issue.record("no reply to write a lot")
            return
        }
        #expect(text.hasSuffix("Paragraph 30 ends here."))
    }

    @Test("ask me asks through a permission prompt; the answer writes the call, the reply and the prompt's end")
    internal func question() async throws {
        let pane = agentPane()
        _ = try await pane.sendPrompt(text: "ask me")
        let (askedId, approval) = try lastPrompt()
        #expect(approval.tool == "AskUserQuestion")
        #expect(approval.questions == FakeChatWorld.shells)
        let question = ChatQuestion(questions: FakeChatWorld.shells, status: .running, output: nil)
        _ = try await pane.answerQuestion(question: question, answers: [QuestionAnswer(picked: ["fish"], other: nil)])
        let answered = told().suffix(3)
        let afterRow: [ChatBody] = [.assistant(text: "Going with fish."), .approvalDone(approval: approval)]
        #expect(Array(answered.map(\.body).dropFirst()) == afterRow)
        #expect(answered.last?.id == askedId)
        guard case .question(let row) = answered.first?.body else {
            Issue.record("no question row")
            return
        }
        #expect(row.status == .ok)
        #expect(row.output == "fish")
    }

    @Test("run the build asks to allow it; Yes runs it and ends the prompt, No does nothing")
    internal func approval() async throws {
        let pane = agentPane()
        _ = try await pane.sendPrompt(text: "run the build")
        let approval = try lastPrompt().approval
        #expect(approval.tool == "Bash")
        let before = told().count
        _ = try await pane.answerApproval(approval: approval, label: "No")
        _ = try await pane.interrupt()
        #expect(told().count == before, "No and Stop tell nothing")
        _ = try await pane.answerApproval(approval: approval, label: "Yes")
        #expect(bodies().suffix(2) == [.assistant(text: "The build passed."), .approvalDone(approval: approval)])
    }

    @Test("ask me, then drop: the link drops, then the catch-up writes the answered call and ends the prompt")
    internal func dropped() async throws {
        let pane = agentPane()
        _ = try await pane.sendPrompt(text: "ask me, then drop")
        let askedId = try lastPrompt().id
        try await AppWindow.until { told().last?.body == .live }
        #expect(core.emitted.contains(.chatState(pane: FakeChatWorld.pane, state: .reconnecting)))
        let ended = told().suffix(2).first
        #expect(ended?.id == askedId)
        guard case .approvalDone = ended?.body else {
            Issue.record("the prompt didn't end: \(String(describing: ended?.body))")
            return
        }
        // a question answered meanwhile has no prompt left to answer
        let question = ChatQuestion(questions: FakeChatWorld.shells, status: .running, output: nil)
        let count = told().count
        _ = try await pane.answerQuestion(question: question, answers: [QuestionAnswer(picked: ["bash"], other: nil)])
        #expect(told().count == count)
    }
}
