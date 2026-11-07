import NgtermDesign
import SwiftUI

/// A session's panes as numbered buttons in the origin's order, in the toolbar (mobile/android's `PaneSwitcher`): the
/// selected one bold and underlined, in the tint.
///
/// VoiceOver reads a button as "Pane 1 of 2" and the pane's title when its program set one, and says which is
/// selected. With one pane there is nothing to switch, and it draws nothing. Where the window can show the split
/// (`split` not ``PaneAlone/Split/fixed``), a tab also toggles it (``PaneAlone``), and VoiceOver's hint says what its
/// tap does: "Shows
/// this pane alone" in the split, "Shows both panes" on the pane shown alone.
internal struct PaneSwitcher: View {
    internal let panes: [String]
    internal let selected: String?
    internal let titles: [String: String]
    internal let onSelect: @MainActor (String) -> Void
    internal var split = PaneAlone.Split.fixed

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
        .accessibilityHint(hint(pane: pane))
        .accessibilityIdentifier("pane-\(number)")
    }

    // what a tap does where it toggles the split
    private func hint(pane: String) -> String {
        switch split {
        case .both:
            String(localized: "Shows this pane alone")

        case .alone where pane == selected:
            String(localized: "Shows both panes")

        default:
            ""
        }
    }

    private func label(number: Int, pane: String) -> String {
        let count = panes.count
        if let title = titles[pane], !title.allSatisfy(\.isWhitespace) {
            return String(localized: "Pane \(number) of \(count), \(title)")
        }
        return String(localized: "Pane \(number) of \(count)")
    }
}
