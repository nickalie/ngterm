import NgtermCore
import NgtermCoreFakes
import NgtermTerminal
import SwiftUI
import Testing

@testable import NgtermFeatures

/// Goldens of a pane's chat (mobile/android's `ChatScreenshotTest`): a conversation with every kind of row, the
/// approval and question cards, the `/` menu, an upload on its way under the account line, and the chat ended.
@MainActor
@Suite("chat snapshots")
internal struct ChatSnapshotTests {
    private typealias Samples = ChatSamples

    private static let diff = [
        "diff --git a/docs/install.md b/docs/install.md",
        "--- a/docs/install.md",
        "+++ b/docs/install.md",
        "@@ -12,3 +12,4 @@ Install ngterm",
        " Put ngtermctl on the PATH of ssh logins.",
        "-Check it with `which ngtermctl` in bash.",
        "+Check it with `command -v ngtermctl`, which works in",
        "+bash, zsh and fish alike.",
    ]
    .joined(separator: "\n")

    private static let conversation: [ChatEvent] = [
        Samples.event("conv", Samples.conversation),
        Samples.event(
            "u1",
            Samples.user("The install guide says to run `which ngtermctl`, which fails on fish. Fix the guide.")
        ),
        Samples.event("th1", .thinking(text: "")),
        Samples.event("a1", Samples.reply("I'll look at the installer's PATH check first, then the guide.")),
        Samples.event(
            "t1",
            Samples.tool("Bash", "grep -n PATH scripts/install.sh", .ok, output: "14:if ! which ngtermctl")
        ),
        Samples.event("t2", Samples.tool("Read", "scripts/install.sh", .ok, kind: .read)),
        Samples.event(
            "t3",
            .tool(
                tool: ChatTool(
                    name: "Edit",
                    kind: .edit,
                    title: "docs/install.md",
                    status: .ok,
                    output: nil,
                    diff: diff,
                    truncated: false
                )
            )
        ),
        Samples.event("task", Samples.tool("Task", "Look for which in the other guides", .ok, kind: .task)),
        Samples.event("s1", Samples.tool("Grep", "which ngtermctl docs/", .ok, kind: .search), parent: "task"),
        Samples.event("t4", Samples.tool("Bash", "npm test", .error, output: "1 failing")),
        Samples.event("n1", .notice(kind: .interrupt, text: "Interrupted by the user", detail: nil)),
        Samples.event(
            "a2",
            Samples.reply(
                """
                Done. The guide and the installer now both use

                ```sh
                command -v ngtermctl
                ```

                which works in **bash**, **zsh** and **fish**.

                - `scripts/install.sh`: the check
                - `docs/install.md`: the step
                """
            )
        ),
        Samples.event("t5", Samples.tool("Bash", "npm run build")),
        Samples.event("live", .live),
    ]

    // a chat over a fake core that is never connected: what it shows is the events given
    private static func chat(_ events: [ChatEvent], working: Bool = false) -> ChatModel {
        let core = FakeMobileCore()
        core.connected = ["m1"]
        core.catalogs[ChatWorld.pane] = [
            Samples.command("compact", "Clear history but keep a summary"),
            Samples.command("context", "Show how the context window is used"),
        ]
        let sources = ChatModel.Sources(
            core: core,
            handle: { nil },
            isWorking: { working },
            connects: { 0 },
            onEnded: { _ in
                // a golden ends nothing
            }
        )
        let chat = ChatModel(pane: ChatWorld.pane, sources: sources) { query, items in
            items.filter { $0.name.hasPrefix(query) }
        }
        chat.take(events)
        return chat
    }

    private static func screen(_ chat: ChatModel) -> some View {
        ChatScreen(
            chat: chat,
            machine: "build-01",
            keyboard: TerminalKeyboard(),
            isSelected: true,
            onShowTerminal: ScreenSnapshot.ignore
        )
    }

    @Test("a conversation: prompts, replies, tools of every kind, a subagent", arguments: ScreenVariant.allCases)
    internal func conversation(variant: ScreenVariant) {
        let chat = Self.chat(Self.conversation, working: true)
        ScreenSnapshot.check(Self.screen(chat), named: "chat-conversation", variant: variant)
    }

    @Test("an approval card with the dialog's options", arguments: [ScreenVariant.light, .dark])
    internal func approval(variant: ScreenVariant) {
        let chat = Self.chat(Self.conversation + [Samples.event("ap1", .approval(approval: Samples.approval))])
        ScreenSnapshot.check(Self.screen(chat), named: "chat-approval", variant: variant)
    }

    @Test("a question card", arguments: [ScreenVariant.light])
    internal func question(variant: ScreenVariant) {
        let chat = Self.chat(Self.conversation + [Samples.event("q1", Samples.question())])
        ScreenSnapshot.check(Self.screen(chat), named: "chat-question", variant: variant)
    }

    @Test("the / menu over the rows, the activity line under it", arguments: [ScreenVariant.light, .dark])
    internal func commandMenu(variant: ScreenVariant) async {
        let chat = Self.chat(Self.conversation, working: true)
        chat.composer.edit("/co")
        await Wait.until { chat.composer.catalog != .fetching }
        ScreenSnapshot.check(Self.screen(chat), named: "chat-commands", variant: variant)
    }

    @Test("the account line and an upload on its way", arguments: [ScreenVariant.light])
    internal func upload(variant: ScreenVariant) {
        let usage = ChatUsage(usedPercent: 82, resetsAt: nil)
        let account = Samples.event("acct", .account(email: "dev@example.com", weekly: usage))
        let chat = Self.chat(Self.conversation + [account])
        chat.snapshot.upload = .sending(name: "shot.png", sent: 42, total: 100, isSaving: false)
        chat.composer.edit("Look at")
        ScreenSnapshot.check(Self.screen(chat), named: "chat-upload", variant: variant)
    }

    @Test("nothing said yet, and the chat ended", arguments: [ScreenVariant.light])
    internal func states(variant: ScreenVariant) {
        let empty = Self.chat([Samples.event("conv", Samples.conversation), Samples.event("live", .live)])
        ScreenSnapshot.check(Self.screen(empty), named: "chat-empty", variant: variant)
        let ended = Self.chat([Samples.event("conv", Samples.conversation)])
        ended.take(.ended)
        ScreenSnapshot.check(Self.screen(ended), named: "chat-ended", variant: variant)
    }
}
