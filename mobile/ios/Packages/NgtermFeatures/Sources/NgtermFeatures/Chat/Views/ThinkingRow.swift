import NgtermDesign
import SwiftUI

/// What the agent thought, collapsed to one quiet line (mobile/android's `ThinkingRow`): Claude writes it empty
/// ("Thinking" then, which opens nothing), Codex a summary title, which a tap shows whole.
internal struct ThinkingRow: View {
    internal let text: String
    internal let isOpen: Bool
    internal let onToggle: @MainActor () -> Void

    internal var body: some View {
        let words = text.replacing("**", with: "").trimmingCharacters(in: .whitespacesAndNewlines)
        ChatLine(
            symbol: ChatStyle.thinkingSymbol,
            glyphLabel: nil,
            title: words.isEmpty ? String(localized: "Thinking") : words,
            titleFont: .subheadline,
            titleColor: Palette.secondaryText.color,
            toggle: words.isEmpty
                ? nil
                : ChatToggle(
                    isExpanded: isOpen,
                    show: String(localized: "Shows all of it"),
                    hide: String(localized: "Shows less"),
                    onToggle: onToggle
                )
        ) {
            EmptyView()
        }
    }
}
