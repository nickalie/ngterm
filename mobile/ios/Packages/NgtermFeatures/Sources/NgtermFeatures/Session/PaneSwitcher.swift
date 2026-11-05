import NgtermDesign
import SwiftUI

/// A session's panes as numbered buttons in the origin's order, in the toolbar (mobile/android's `PaneSwitcher`): the
/// selected one bold and underlined, in the tint.
///
/// VoiceOver reads a button as "Pane 1 of 2" and the pane's title when its program set one, and says which is
/// selected. With one pane there is nothing to switch, and it draws nothing.
internal struct PaneSwitcher: View {
    internal let panes: [String]
    internal let selected: String?
    internal let titles: [String: String]
    internal let onSelect: @MainActor (String) -> Void

    internal var body: some View {
        if panes.count > 1 {
            HStack(spacing: 0) {
                ForEach(Array(panes.enumerated()), id: \.element) { index, pane in
                    tab(number: index + 1, pane: pane)
                }
            }
        }
    }

    private func tab(number: Int, pane: String) -> some View {
        let isSelected = pane == selected
        return Button {
            onSelect(pane)
        } label: {
            Text(verbatim: "\(number)")
                .font(.headline.monospacedDigit().weight(isSelected ? .bold : .regular))
                .foregroundStyle(isSelected ? Palette.tint.color : Palette.secondaryText.color)
                .underline(isSelected)
                .frame(minWidth: Spacing.hitTarget, minHeight: Spacing.hitTarget)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label(number: number, pane: pane))
        .accessibilityAddTraits(isSelected ? .isSelected : [])
        .accessibilityIdentifier("pane-\(number)")
    }

    private func label(number: Int, pane: String) -> String {
        let count = panes.count
        if let title = titles[pane], !title.allSatisfy(\.isWhitespace) {
            return String(localized: "Pane \(number) of \(count), \(title)")
        }
        return String(localized: "Pane \(number) of \(count)")
    }
}
