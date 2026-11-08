import Foundation
import NgtermCore
import Testing

@testable import NgtermFeatures

/// What the chat says about what it shows (mobile/android's `ChatStyle` and `ProblemBanner` words): why an action did
/// nothing and what the banner offers, an upload's name, the words VoiceOver reads for a row's glyph, the rows left
/// open in a conversation, a reply's later runs, and a multi-select question's draft.
@MainActor
@Suite("the chat's words")
internal struct ChatWordsTests {
    private static let shells = ChatQuestionItem(
        question: "Which shells should the guide cover?",
        header: "Shells",
        multiSelect: true,
        options: [
            ChatQuestionOption(label: "bash", description: nil),
            ChatQuestionOption(label: "fish", description: nil),
        ]
    )

    private static func tool(name: String, title: String) -> ChatTool {
        ChatTool(name: name, kind: .other, title: title, status: .ok, output: nil, diff: nil, truncated: false)
    }

    @Test("what the core answered becomes the problem it reports, none when the action was sent")
    internal func problemFromTheCore() {
        #expect(ChatProblem(AgentInputOutcome.sent) == nil)
        #expect(ChatProblem(AgentInputOutcome.notOnScreen) == .notOnScreen)
        #expect(ChatProblem(AgentInputOutcome.dialogOpen) == .dialogOpen)
        #expect(ChatProblem(AgentInputOutcome.leadRefused) == .leadRefused)
        #expect(ChatProblem(AgentInputOutcome.unconfirmed) == .unconfirmed)
        #expect(ChatProblem(PaneInputError.Closed) == .closed)
        #expect(ChatProblem(PaneInputError.Backpressure) == .notReading)
        #expect(ChatProblem(PaneInputError.Refused(reason: "no such pane")) == .refused(reason: "no such pane"))
    }

    @Test("the banner offers the terminal where it is the way on, in the attention tone where the user can act")
    internal func problemBanner() {
        let all: [ChatProblem] = [
            .closed, .dialogOpen, .leadRefused, .notOnScreen, .notReading, .refused(reason: "busy"), .unconfirmed,
        ]
        #expect(all.filter(\.offersTerminal) == [.notOnScreen, .unconfirmed])
        #expect(all.filter(\.isActionable) == [.dialogOpen, .notOnScreen, .unconfirmed])
        let messages = all.map(ChatProblemBanner.message)
        #expect(Set(messages).count == all.count, "each problem says its own thing")
        #expect(ChatProblemBanner.message(.refused(reason: "busy")) == "Not sent: busy")
        #expect(
            ChatProblemBanner.message(.closed) == "This pane isn't on screen any more, so nothing was sent."
        )
        #expect(
            ChatProblemBanner.message(.notReading)
                == "The program isn't reading its input. Nothing was sent; try again in a moment."
        )
        #expect(
            ChatProblemBanner.message(.leadRefused) == "Another device kept control of this pane, so nothing was sent."
        )
    }

    @Test("an upload keeps the name the user picked, on its way or stopped short")
    internal func upload() {
        let sending = ChatUpload.sending(name: "shot.png", sent: 1, total: 2, isSaving: false)
        let failed = ChatUpload.failed(name: "film.mov", problem: .tooLarge)
        #expect(sending.name == "shot.png")
        #expect(sending.isSending)
        #expect(failed.name == "film.mov")
        #expect(!failed.isSending)
    }

    @Test("the agent's name, and the answers an approval offers before the screen is read, as its dialogs word them")
    internal func agentWords() {
        #expect(ChatStyle.agentName("claude") == "Claude Code")
        #expect(ChatStyle.agentName("codex") == "Codex")
        #expect(ChatStyle.agentName("gemini") == "The agent")
        #expect(ChatStyle.agentName(nil) == "The agent")
        let codex = ["Yes, proceed", "No, and tell Codex what to do differently"]
        #expect(ChatStyle.genericApprovalLabels("codex") == codex)
        #expect(ChatStyle.genericApprovalLabels("claude") == ["Yes", "No"])
        #expect(ChatStyle.genericApprovalLabels(nil) == ["Yes", "No"])
    }

    @Test("each kind of tool, notice and command has a glyph and a word of its own")
    internal func kindWords() {
        let tools: [ChatToolKind] = [.shell, .edit, .read, .search, .web, .task, .other]
        #expect(Set(tools.map { ChatStyle.symbol($0) }).count == tools.count)
        #expect(
            tools.map { ChatStyle.word($0) } == ["Command", "Edit", "Read", "Search", "Web", "Subagent", "Tool"]
        )
        let notices: [ChatNoticeKind] = [.compaction, .interrupt, .apiError, .summary]
        #expect(Set(notices.map { ChatStyle.symbol($0) }).count == notices.count)
        #expect(
            notices.map { ChatStyle.word($0) } == ["Context compacted", "Interrupted", "API error", "Summary"]
        )
        let commands: [AgentCommandKind] = [.builtin, .skill, .command, .plugin]
        #expect(commands.map { ChatStyle.word($0) } == ["Built-in", "Skill", "Command", "Plugin"])
    }

    @Test("a tool's line is its title, else its name, else that it is an earlier call; its steps are counted")
    internal func toolLine() {
        #expect(ChatStyle.title(Self.tool(name: "Bash", title: "npm test")) == "npm test")
        #expect(ChatStyle.title(Self.tool(name: "Bash", title: " ")) == "Bash")
        #expect(ChatStyle.title(Self.tool(name: "", title: "")) == "An earlier tool call")
        #expect(ChatStyle.steps(1) == "1 step")
        #expect(ChatStyle.steps(3) == "3 steps")
    }

    @Test("a row opens and closes in its conversation, and what was open in another opens nothing here")
    internal func openRows() {
        var open = OpenRows(conversation: "c1")
        #expect(open.of("c1").isEmpty)
        open = open.toggled("c1", "e:t1")
        open = open.toggled("c1", "e:t2")
        #expect(open.of("c1") == ["e:t1", "e:t2"])
        #expect(open.of("c2").isEmpty, "another conversation's rows open nothing")
        open = open.toggled("c1", "e:t1")
        #expect(open.of("c1") == ["e:t2"])
        open = open.toggled("c2", "e:t2")
        #expect(open.of("c2") == ["e:t2"], "a key that came again opens in its own conversation")
        #expect(open.of("c1").isEmpty)
    }

    @Test("a block's later runs go on its text, whatever the block")
    internal func laterRuns() {
        let quote = PresentationIntent(.blockQuote, identity: 1)
        let list = PresentationIntent(.unorderedList, identity: 2)
        let item = PresentationIntent(.listItem(ordinal: 1), identity: 3, parent: list)
        let intents = [
            PresentationIntent(.paragraph, identity: 4),
            PresentationIntent(.header(level: 3), identity: 5),
            PresentationIntent(.codeBlock(languageHint: nil), identity: 6),
            PresentationIntent(.paragraph, identity: 7, parent: item),
            PresentationIntent(.paragraph, identity: 8, parent: quote),
            PresentationIntent(.thematicBreak, identity: 9),
        ]
        var reader = MarkdownBlockReader()
        for intent in intents {
            reader.take(AttributedString("one"), intent: intent)
            reader.take(AttributedString(" two"), intent: intent)
        }
        let both = AttributedString("one two")
        #expect(
            reader.done() == [
                .paragraph(both), .heading(level: 3, both), .code("one two"),
                .item(marker: MarkdownBlock.bullet, depth: 0, both), .quote(both), .rule,
            ]
        )
    }

    @Test("a multi-select question holds any options, and the typed text with them once it is picked")
    internal func multiPick() {
        let question = ChatQuestion(questions: [Self.shells], status: .running, output: nil)
        var draft = QuestionDraft(for: question)
        draft = draft.toggling(0, Self.shells, label: "bash").toggling(0, Self.shells, label: "fish")
        #expect(draft.answers(for: question) == [QuestionAnswer(picked: ["bash", "fish"], other: nil)])
        draft = draft.toggling(0, Self.shells, label: "bash")
        #expect(draft.answers(for: question) == [QuestionAnswer(picked: ["fish"], other: nil)], "a second tap lets go")
        draft = draft.typing(0, Self.shells, text: " zsh ")
        #expect(draft.answers(for: question) == [QuestionAnswer(picked: ["fish"], other: "zsh")])
        draft = draft.togglingOther(0, Self.shells)
        #expect(draft.answers(for: question) == [QuestionAnswer(picked: ["fish"], other: nil)], "the typed row let go")
        draft = draft.togglingOther(0, Self.shells)
        #expect(draft.answers(for: question) == [QuestionAnswer(picked: ["fish"], other: "zsh")])
        #expect(draft.fits(question))
    }

    @Test("on a single-select question, picking the typed row again lets the option go")
    internal func singleTypedRow() {
        let question = ChatSamples.shells()
        let item = question.questions[0]
        let draft = QuestionDraft(for: question)
            .typing(0, item, text: "zsh")
            .toggling(0, item, label: "bash")
        #expect(draft.answers(for: question) == [QuestionAnswer(picked: ["bash"], other: nil)])
        let typed = draft.togglingOther(0, item)
        #expect(typed.answers(for: question) == [QuestionAnswer(picked: [], other: "zsh")])
        #expect(typed.togglingOther(0, item) == typed, "the one pick stays picked")
    }
}
