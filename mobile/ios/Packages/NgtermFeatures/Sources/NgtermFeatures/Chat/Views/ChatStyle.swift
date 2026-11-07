import Foundation
import NgtermCore

/// The chat's words and glyphs (mobile/android's `ChatStyle.kt`), as SF Symbols.
internal enum ChatStyle {
    /// The chat's own glyph: a speech bubble with lines.
    internal static let chatSymbol = "text.bubble"
    /// What a thinking line shows.
    internal static let thinkingSymbol = "ellipsis"
    /// Attach: a paperclip.
    internal static let attachSymbol = "paperclip"
    /// Send: the return key's arrow, as a prompt goes in with Return in the terminal.
    internal static let sendSymbol = "arrow.turn.down.left"
    /// The most of a reading line a prompt's bubble takes.
    internal static let bubbleWidth: CGFloat = 480
    /// The room before a line's title (its padding, glyph and gap), where what it opens lines up.
    internal static let detailsIndent: CGFloat = 30
    /// A line's glyph's box.
    internal static let glyphSize: CGFloat = 18
    /// A line's glyph's own size.
    internal static let glyphPoints: CGFloat = 14
    /// The rule a subagent's steps and a reply's quote sit behind.
    internal static let ruleWidth: CGFloat = 2

    /// The agent's name as people know it, from its wire name (`claude`, `codex`).
    internal static func agentName(_ agent: String?) -> String {
        switch agent {
        case "claude":
            String(localized: "Claude Code")

        case "codex":
            String(localized: "Codex")

        default:
            String(localized: "The agent")
        }
    }

    /// The answers an approval card offers before the pane's screen is read.
    ///
    /// Every dialog of the agent has them, worded as its dialog words them, so a tap finds its option once the screen
    /// is read (crates/ngterm-mobile, `fallback_labels`).
    internal static func genericApprovalLabels(_ agent: String?) -> [String] {
        agent == "codex" ? ["Yes, proceed", "No, and tell Codex what to do differently"] : ["Yes", "No"]
    }

    /// The glyph a tool's kind shows at the start of its line.
    internal static func symbol(_ kind: ChatToolKind) -> String {
        switch kind {
        case .shell:
            "apple.terminal"

        case .edit:
            "pencil"

        case .read:
            "doc.text"

        case .search:
            "magnifyingglass"

        case .web:
            "globe"

        case .task:
            "arrow.triangle.branch"

        case .other:
            "wrench.adjustable"
        }
    }

    /// The word VoiceOver reads for a tool's kind, the glyph having no word beside it.
    internal static func word(_ kind: ChatToolKind) -> String {
        switch kind {
        case .shell:
            String(localized: "Command")

        case .edit:
            String(localized: "Edit")

        case .read:
            String(localized: "Read")

        case .search:
            String(localized: "Search")

        case .web:
            String(localized: "Web")

        case .task:
            String(localized: "Subagent")

        case .other:
            String(localized: "Tool")
        }
    }

    /// A notice's glyph: an interrupt a square, a compaction two arrows meeting, an API error the alert, a summary the
    /// page.
    internal static func symbol(_ kind: ChatNoticeKind) -> String {
        switch kind {
        case .compaction:
            "arrow.down.right.and.arrow.up.left"

        case .interrupt:
            "stop.fill"

        case .apiError:
            "exclamationmark.triangle"

        case .summary:
            "doc.plaintext"
        }
    }

    /// A notice's word.
    internal static func word(_ kind: ChatNoticeKind) -> String {
        switch kind {
        case .compaction:
            String(localized: "Context compacted")

        case .interrupt:
            String(localized: "Interrupted")

        case .apiError:
            String(localized: "API error")

        case .summary:
            String(localized: "Summary")
        }
    }

    /// The word a command's kind shows at the end of its row in the `/` menu.
    internal static func word(_ kind: AgentCommandKind) -> String {
        switch kind {
        case .builtin:
            String(localized: "Built-in")

        case .skill:
            String(localized: "Skill")

        case .command:
            String(localized: "Command")

        case .plugin:
            String(localized: "Plugin")
        }
    }

    /// A tool's one line: its title, else its name, else that it is an earlier call.
    internal static func title(_ tool: ChatTool) -> String {
        if !AskText.isBlank(tool.title) {
            return tool.title
        }
        return AskText.isBlank(tool.name) ? String(localized: "An earlier tool call") : tool.name
    }

    /// A count of steps: "1 step", "2 steps".
    internal static func steps(_ count: Int) -> String {
        count == 1 ? String(localized: "1 step") : String(localized: "\(count) steps")
    }
}
