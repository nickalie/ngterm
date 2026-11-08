import NgtermCore
import NgtermDesign
import SwiftUI
import Testing

@testable import NgtermFeatures

/// Goldens of what opens over the composer (mobile/android's `SlashCommandMenu` and `ActivityStrip`): the `/` menu with
/// one match and its argument hint, with none, while the commands are on their way and when the origin lists none; and
/// the activity line of a working agent opened to the calls still running, its Stop held while an action is on its way.
@MainActor
@Suite("chat menus snapshots")
internal struct ChatMenusSnapshotTests {
    private static let review = AgentCommand(
        name: "review",
        description: "Reviews the branch's changes",
        kind: .skill,
        argumentHint: "[branch]"
    )

    private static func pickNothing(_: AgentCommand) {
        // a golden picks nothing
    }

    private static func tool(_ name: String, _ title: String, kind: ChatToolKind) -> ChatTool {
        ChatTool(name: name, kind: kind, title: title, status: .running, output: nil, diff: nil, truncated: false)
    }

    // in a scroll view, as the chat's foot holds them: each at its own height
    private static func column(@ViewBuilder _ content: () -> some View) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.large) {
                content()
            }
            .padding(Spacing.large)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(ChatGrounds.chat)
    }

    @Test("the / menu: one match with its hint, none, the commands on their way, and none listed")
    internal func menus() {
        let view = Self.column {
            SlashCommandMenu(
                menu: .commands(query: "rev", commands: [Self.review], highlighted: 0),
                agent: "codex",
                onPick: Self.pickNothing
            )
            SlashCommandMenu(
                menu: .commands(query: "deploy", commands: [], highlighted: 0),
                agent: "claude",
                onPick: Self.pickNothing
            )
            SlashCommandMenu(menu: .waiting(query: "", isFetching: true), agent: "claude", onPick: Self.pickNothing)
            SlashCommandMenu(menu: .waiting(query: "", isFetching: false), agent: nil, onPick: Self.pickNothing)
        }
        ScreenSnapshot.check(view, named: "chat-menus", variant: .light)
    }

    @Test("the activity line of a working agent opened to its calls still running, its Stop held")
    internal func activity() {
        let running = [
            Self.tool("Task", "Look for which in the guides", kind: .task),
            Self.tool("Bash", "npm run build", kind: .shell),
            Self.tool("WebFetch", "https://pubs.opengroup.org", kind: .web),
        ]
        let view = Self.column {
            ActivityStrip(
                isWorking: true,
                running: running,
                agent: "Codex",
                onStop: {
                    // a golden stops nothing
                },
                isStopEnabled: false,
                isExpanded: .constant(true)
            )
        }
        ScreenSnapshot.check(view, named: "chat-activity", variant: .light)
    }
}
