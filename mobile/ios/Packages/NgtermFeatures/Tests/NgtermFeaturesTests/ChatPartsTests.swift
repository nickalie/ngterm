import Foundation
import NgtermCore
import Testing

@testable import NgtermFeatures

/// The chat's parts that hold no state of their own (mobile/android's `ChatPartsTest`): a question card's draft, a
/// diff's lines, a reset's words, the activity line's counts.
@MainActor
@Suite("the chat's parts")
internal struct ChatPartsTests {
    private let question = ChatSamples.shells()

    private var item: ChatQuestionItem {
        question.questions[0]
    }

    @Test("a single-select question holds one pick: an option, or the typed text, which typing picks")
    internal func singlePick() {
        var draft = QuestionDraft(for: question)
        #expect(draft.answers(for: question).isEmpty, "nothing picked: nothing to send")
        draft = draft.toggling(0, item, label: "bash")
        #expect(draft.answers(for: question) == [QuestionAnswer(picked: ["bash"], other: nil)])
        draft = draft.typing(0, item, text: "zsh")
        #expect(draft.answers(for: question) == [QuestionAnswer(picked: [], other: "zsh")])
        draft = draft.typing(0, item, text: " ")
        #expect(draft.answers(for: question).isEmpty, "clearing the text lets the row go")
        #expect(draft.togglingOther(0, item) == draft, "the typed row takes a tap only while it holds text")
        #expect(draft.fits(question))
        #expect(!QuestionDraft(for: ChatQuestion(questions: [], status: .running, output: nil)).fits(question))
    }

    @Test("a diff's lines: headers until a hunk, then each by its first character, a new file once a hunk is whole")
    internal func diffLines() {
        let diff = [
            "diff --git a/a.md b/a.md", "--- a/a.md", "+++ b/a.md", "@@ -1,2 +1,2 @@", " kept", "-old", "+new",
            "--- a/b.md", "+++ b/b.md", "@@ -1 +1 @@", "-x", "+y", "",
        ]
        .joined(separator: "\n")
        let lines = DiffLines(diff)
        #expect(
            lines.lines.map(\.kind) == [
                .file, .file, .file, .hunk, .context, .removed, .added, .file, .file, .hunk, .removed, .added,
            ]
        )
        #expect(lines.more == 0)
        let long = DiffLines((0..<5).map { "+\($0)" }.joined(separator: "\n"), limit: 3)
        #expect(long.lines.count == 3)
        #expect(long.more == 2)
    }

    @Test("a reset within six days reads as a weekday and a time, later as a date")
    internal func resetsAt() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC") ?? .gmt
        calendar.locale = Locale(identifier: "en_US")
        let now = Date(timeIntervalSince1970: 1_794_000_000)
        let soon = AccountLine.resetsAt(now.addingTimeInterval(3_600), now: now, calendar: calendar)
        #expect(soon.split(separator: " ").count > 1, "\(soon): a weekday and a time")
        let later = AccountLine.resetsAt(now.addingTimeInterval(864_000), now: now, calendar: calendar)
        #expect(later.contains("2026"), "\(later): a date")
    }

    @Test("the activity line counts subagents, then commands, then other tools")
    internal func counts() {
        let running: [ChatTool] = [("Task", ChatToolKind.task), ("Bash", .shell), ("Bash", .shell)].map { name, kind in
            ChatTool(name: name, kind: kind, title: name, status: .running, output: nil, diff: nil, truncated: false)
        }
        #expect(ActivityStrip.counts(running) == "1 subagent, 2 commands")
    }

    @Test("a reply's Markdown reads as its blocks: headings, paragraphs, lists, code, a table")
    internal func markdown() {
        let blocks = MarkdownBlock.blocks(
            of: """
                # Done

                The guide uses **one** check.

                - first
                - second

                ```sh
                command -v ngtermctl
                ```

                | a | b |
                |---|---|
                | 1 | 2 |
                """
        )
        let kinds = blocks.map { block -> String in
            switch block {
            case .heading:
                "h"

            case .paragraph:
                "p"

            case .item(let marker, _, _):
                marker

            case .code(let code):
                "code:\(code)"

            case .table(let rows, let hasHeader):
                "table:\(rows.count):\(hasHeader)"

            case .quote:
                "q"

            case .rule:
                "-"
            }
        }
        #expect(kinds == ["h", "p", "•", "•", "code:command -v ngtermctl", "table:2:true"])
    }
}
