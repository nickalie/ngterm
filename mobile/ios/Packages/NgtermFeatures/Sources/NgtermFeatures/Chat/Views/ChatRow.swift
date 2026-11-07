import NgtermCore
import NgtermDesign
import SwiftUI

/// A row of the chat, by what it is, with its subagent's steps under a task (mobile/android's `ChatRow`); `expanded`
/// holds the open rows' keys.
internal struct ChatRow: View {
    internal let item: ChatItem
    internal let expanded: Set<String>
    internal let onToggle: @MainActor (String) -> Void

    internal var body: some View {
        let isOpen = expanded.contains(item.key)
        let key = item.key
        let toggle: @MainActor @Sendable () -> Void = { [onToggle] in onToggle(key) }
        switch item {
        case .divider:
            ConversationDivider()

        case .subagent(_, let children):
            SubagentRow(steps: children.count, isOpen: isOpen, onToggle: toggle) {
                steps(children)
            }

        case .entry(_, let body, let children):
            entry(body, children: children, isOpen: isOpen, toggle: toggle)
        }
    }

    @ViewBuilder
    private func entry(
        _ body: ChatBody,
        children: [ChatItem],
        isOpen: Bool,
        toggle: @escaping @MainActor () -> Void
    ) -> some View {
        switch body {
        case .user(let text, let images):
            PromptBubble(text: text, images: Int(images))

        case .assistant(let text):
            MarkdownText(text)

        case .thinking(let text):
            ThinkingRow(text: text, isOpen: isOpen, onToggle: toggle)

        case .tool(let tool):
            ToolRow(tool: tool, steps: children.count, isOpen: isOpen, onToggle: toggle) {
                steps(children)
            }

        case .question(let question):
            QuestionRow(question: question, isOpen: isOpen, onToggle: toggle)

        case .notice(let kind, let text, let detail):
            NoticeRow(kind: kind, text: text, detail: detail, isOpen: isOpen, onToggle: toggle)

        default:
            // the rest is the chat model's, never a row
            EmptyView()
        }
    }

    // a subagent's steps, every kind of row, behind a rule at their start
    private func steps(_ children: [ChatItem]) -> some View {
        HStack(alignment: .top, spacing: Spacing.small) {
            Rectangle()
                .fill(Color(uiColor: .separator))
                .frame(width: ChatStyle.ruleWidth)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 0) {
                ForEach(children) { child in
                    Self(item: child, expanded: expanded, onToggle: onToggle)
                }
            }
        }
    }
}
