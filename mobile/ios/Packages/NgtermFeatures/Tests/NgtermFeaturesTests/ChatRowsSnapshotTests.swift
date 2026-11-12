import NgtermCore
import NgtermCoreFakes
import NgtermDesign
import NgtermTerminal
import SwiftUI
import Testing

@testable import NgtermFeatures

/// Goldens of a chat's rows opened, as a tap leaves them (mobile/android's `ChatRowsScreenshotTest`), of a reply's
/// Markdown, of the strips and lines over and under the rows, of the approval card's detail, of a diff past what is
/// drawn, and of the chat while its backlog is on its way.
///
/// The rows are drawn in a plain column: the chat's own list draws only the rows in sight.
@MainActor
@Suite("chat rows snapshots")
internal struct ChatRowsSnapshotTests {
    private typealias Samples = ChatSamples

    private static let diff = [
        "--- a/docs/install.md",
        "+++ b/docs/install.md",
        "@@ -12,2 +12,2 @@ Install ngterm",
        " Put ngtermctl on the PATH of ssh logins.",
        "-Check it with `which ngtermctl` in bash.",
        "+Check it with `command -v ngtermctl`.",
    ]
    .joined(separator: "\n")

    private static let reply = """
        # Install guide

        The check now uses `command -v`, which [POSIX](https://pubs.opengroup.org) defines.

        ## What changed

        ### In the installer

        > The old check failed on fish.

        1. The installer's check
            - in `scripts/install.sh`
        2. The guide's step

        ```sh
        command -v ngtermctl
        ```

        | Shell | Works |
        | --- | --- |
        | bash | yes |
        | fish | yes |

        ---

        That's all.
        """

    private static func tool(
        _ name: String,
        _ title: String,
        kind: ChatToolKind,
        status: ChatToolStatus = .ok,
        output: String? = nil,
        diff: String? = nil
    ) -> ChatBody {
        .tool(
            tool: ChatTool(
                name: name,
                kind: kind,
                title: title,
                status: status,
                output: output,
                diff: diff,
                truncated: output != nil
            )
        )
    }

    private static func entry(_ id: String, _ body: ChatBody, children: [ChatItem] = []) -> ChatItem {
        .entry(id: id, body: body, children: children)
    }

    // the rows in a plain column, every one of them open
    private static func rows(_ items: [ChatItem]) -> some View {
        let open = Set(items.flatMap { [$0.key] + $0.steps.map(\.key) })
        return column {
            ForEach(items) { item in
                ChatRow(item: item, expanded: open) { _ in
                    // a golden opens nothing more
                }
            }
        }
    }

    // in a scroll view, as the chat's list holds its rows: each at its own height
    private static func column(@ViewBuilder _ content: () -> some View) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.small) {
                content()
            }
            .padding(Spacing.large)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(ChatGrounds.chat)
    }

    // a chat over a fake core that is never connected, of a Claude Code conversation
    private static func chat(_ events: [ChatEvent]) -> ChatModel {
        let core = FakeMobileCore()
        core.connected = ["m1"]
        let sources = ChatModel.Sources(
            core: core,
            handle: { nil },
            isWorking: { false },
            connects: { 0 },
            onEnded: { _ in
                // a golden ends nothing
            }
        )
        let chat = ChatModel(pane: ChatWorld.pane, sources: sources) { _, items in items }
        chat.take(events)
        return chat
    }

    private static func screen(_ chat: ChatModel) -> some View {
        ChatScreen(
            chat: chat,
            machine: "build-01",
            keyboard: TerminalKeyboard(),
            isSelected: false,
            onShowTerminal: ScreenSnapshot.ignore
        )
    }

    @Test("tools opened: an edit's diff, a command's cut output, a task's steps and a subagent's earlier task")
    internal func tools() {
        let grep = Self.entry("s1", Self.tool("Grep", "which ngtermctl docs/", kind: .search))
        let read = Self.entry("s2", Self.tool("Read", "docs/upgrade.md", kind: .read))
        let items: [ChatItem] = [
            .divider,
            Self.entry("t1", Self.tool("Edit", "docs/install.md", kind: .edit, diff: Self.diff)),
            Self.entry(
                "t2",
                Self.tool("Bash", "npm test", kind: .shell, status: .error, output: "1 failing\n  install guide\n")
            ),
            Self.entry("task", Self.tool("Task", "Look for which in the guides", kind: .task), children: [grep]),
            .subagent(parent: "old", children: [read]),
            Self.entry("t3", Self.tool("", " ", kind: .web, status: .running)),
        ]
        ScreenSnapshot.check(Self.rows(items), named: "chat-rows-tools", variant: .light)
    }

    @Test("notes opened: a summary's thought, an answered question, a compaction's summary, an API error")
    internal func notes() {
        let answered = ChatQuestion(
            questions: Samples.shells().questions,
            status: .ok,
            output: "User answered: fish"
        )
        let items: [ChatItem] = [
            Self.entry("th1", .thinking(text: "**Checking** the installer's PATH test")),
            Self.entry("th2", .thinking(text: "")),
            Self.entry("q1", .question(question: answered)),
            Self.entry(
                "n1",
                .notice(kind: .compaction, text: "", detail: "Kept: the guide's fix and the failing test.")
            ),
            Self.entry("n2", .notice(kind: .apiError, text: "Overloaded (529)", detail: nil)),
            Self.entry("n3", .notice(kind: .summary, text: "Fixing the install guide", detail: " ")),
        ]
        ScreenSnapshot.check(Self.rows(items), named: "chat-rows-notes", variant: .light)
    }

    @Test("a reply's Markdown: headings, a quote, nested lists, code, a table and a rule")
    internal func markdown() {
        ScreenSnapshot.check(
            Self.column { MarkdownText(Self.reply) },
            named: "chat-markdown",
            variant: .light
        )
    }

    @Test("the strips over the rows: the link coming back, the problems, the top of the list, sending")
    internal func strips() {
        let view = Self.column {
            ChatReconnectingStrip(machine: "build-01")
            ChatProblemBanner(problem: .notOnScreen, onShowTerminal: ScreenSnapshot.ignore) {
                // a golden dismisses nothing
            }
            ChatProblemBanner(problem: .leadRefused, onShowTerminal: ScreenSnapshot.ignore) {
                // a golden dismisses nothing
            }
            OlderHeader(older: .loading, cwd: nil, onRetry: ScreenSnapshot.ignore)
            OlderHeader(older: .atStart, cwd: "~/docs-site", onRetry: ScreenSnapshot.ignore)
            OlderHeader(older: .failed(reason: "the link went"), cwd: nil, onRetry: ScreenSnapshot.ignore)
            ChatSending()
        }
        ScreenSnapshot.check(view, named: "chat-strips", variant: .light)
    }

    @Test("an upload stopped short for each reason, saved, and the switch to the other view")
    internal func uploads() {
        let view = Self.column {
            UploadLine(
                upload: .failed(name: "film.mov", problem: .tooLarge),
                onCancel: ScreenSnapshot.ignore,
                onDismiss: ScreenSnapshot.ignore
            )
            UploadLine(
                upload: .failed(name: "scan.pdf", problem: .unreadable),
                onCancel: ScreenSnapshot.ignore,
                onDismiss: ScreenSnapshot.ignore
            )
            UploadLine(
                upload: .failed(name: "shot.png", problem: .refused(reason: "the disk is full")),
                onCancel: ScreenSnapshot.ignore,
                onDismiss: ScreenSnapshot.ignore
            )
            UploadLine(
                upload: .sending(name: "shot.png", sent: 100, total: 100, isSaving: true),
                onCancel: ScreenSnapshot.ignore,
                onDismiss: ScreenSnapshot.ignore
            )
            UploadLine(
                upload: .sending(name: "empty.txt", sent: 0, total: 0, isSaving: false),
                onCancel: ScreenSnapshot.ignore,
                onDismiss: ScreenSnapshot.ignore
            )
            ViewSwitch(view: .chat) { _ in
                // a golden switches nothing
            }
            ViewSwitch(view: .terminal) { _ in
                // a golden switches nothing
            }
        }
        ScreenSnapshot.check(view, named: "chat-uploads", variant: .light)
    }

    @Test("approval cards before the screen is read: Codex's answers over a diff, and a command's output")
    internal func approvalDetail() {
        let edit = ChatApproval(
            tool: "Edit",
            title: "docs/install.md",
            detail: Self.diff,
            toolUseId: "toolu_1",
            cwd: "/home/user/project",
            questions: []
        )
        let fetch = ChatApproval(
            tool: "WebFetch",
            title: " ",
            detail: "GET https://pubs.opengroup.org/onlinepubs/9799919799/utilities/command.html",
            toolUseId: "toolu_2",
            cwd: "/home/user/project",
            questions: []
        )
        let view = Self.column {
            ApprovalCardView(
                card: ApprovalCard(id: "ap1", approval: edit, labels: []),
                agent: "codex",
                isBusy: false,
                maxHeight: ScreenVariant.size.height
            ) { _ in
                // a golden answers nothing
            }
            ApprovalCardView(
                card: ApprovalCard(id: "ap2", approval: fetch, labels: ["Yes", "Yes, and don't ask again", "No"]),
                agent: "claude",
                isBusy: true,
                maxHeight: ScreenVariant.size.height
            ) { _ in
                // a golden answers nothing
            }
        }
        ScreenSnapshot.check(view, named: "chat-approval-detail", variant: .light)
    }

    @Test("a diff past what is drawn says, at its end, how many lines are left out")
    internal func longDiff() {
        let added = (1...(DiffLines.limit + 1)).map { "+line \($0)" }
        let long = (["@@ -0,0 +1,\(added.count) @@"] + added).joined(separator: "\n")
        let view = ScrollView {
            DiffView(diff: long)
                .padding(Spacing.large)
        }
        .defaultScrollAnchor(.bottom)
        .background(ChatGrounds.chat)
        ScreenSnapshot.check(view, named: "chat-long-diff", variant: .light)
    }

    @Test("the chat while its backlog is on its way")
    internal func loading() {
        ScreenSnapshot.check(Self.screen(Self.chat([])), named: "chat-loading", variant: .light)
    }
}
