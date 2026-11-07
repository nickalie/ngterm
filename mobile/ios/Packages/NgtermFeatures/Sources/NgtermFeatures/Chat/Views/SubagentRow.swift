import SwiftUI

/// A subagent's steps whose task is older than what the chat has read (mobile/android's `SubagentRow`): a task line
/// with no status of its own, its steps under it.
internal struct SubagentRow<Steps: View>: View {
    internal let steps: Int
    internal let isOpen: Bool
    internal let onToggle: @MainActor () -> Void
    @ViewBuilder internal let children: () -> Steps

    internal var body: some View {
        ChatLine(
            symbol: ChatStyle.symbol(.task),
            glyphLabel: ChatStyle.word(.task),
            title: String(localized: "A subagent's earlier task"),
            titleFont: .subheadline,
            subtitle: ChatStyle.steps(steps),
            toggle: ChatToggle(
                isExpanded: isOpen,
                show: String(localized: "Shows the steps"),
                hide: String(localized: "Hides the steps"),
                onToggle: onToggle
            )
        ) {
            children()
        }
    }
}
