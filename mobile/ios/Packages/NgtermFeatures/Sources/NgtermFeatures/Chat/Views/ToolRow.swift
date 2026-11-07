import NgtermCore
import NgtermDesign
import SwiftUI

/// A tool call's line (mobile/android's `ToolRow`): its kind's glyph, its title in the terminal's face (its name when
/// the title isn't known), and its status.
///
/// When it has a diff, output or a subagent's steps (a task), a tap opens them: the diff, the output, a note when
/// either was cut, then the steps behind a rule.
internal struct ToolRow<Steps: View>: View {
    internal let tool: ChatTool
    internal let steps: Int
    internal let isOpen: Bool
    internal let onToggle: @MainActor () -> Void
    @ViewBuilder internal let children: () -> Steps

    internal var body: some View {
        let opens = tool.diff != nil || tool.output != nil || steps > 0
        let toggle: ChatToggle? =
            if !opens {
                nil
            } else if tool.kind == .task {
                ChatToggle(
                    isExpanded: isOpen,
                    show: String(localized: "Shows the steps"),
                    hide: String(localized: "Hides the steps"),
                    onToggle: onToggle
                )
            } else {
                ChatToggle(
                    isExpanded: isOpen,
                    show: String(localized: "Shows the output"),
                    hide: String(localized: "Hides the output"),
                    onToggle: onToggle
                )
            }
        ChatLine(
            symbol: ChatStyle.symbol(tool.kind),
            glyphLabel: ChatStyle.word(tool.kind),
            title: ChatStyle.title(tool),
            subtitle: steps > 0 ? ChatStyle.steps(steps) : nil,
            status: .tool(tool.status),
            toggle: toggle
        ) {
            if let diff = tool.diff {
                DiffView(diff: diff)
            }
            if let output = tool.output, !output.isEmpty {
                OutputPlate(text: output)
            }
            if tool.truncated {
                Text(String(localized: "Cut short here: the terminal has the rest."))
                    .font(.footnote)
                    .foregroundStyle(Palette.secondaryText.color)
            }
            if steps > 0 {
                children()
            }
        }
    }
}
