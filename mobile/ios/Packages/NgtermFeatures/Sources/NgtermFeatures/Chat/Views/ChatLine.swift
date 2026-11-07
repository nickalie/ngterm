import NgtermDesign
import SwiftUI

/// One line of the chat that isn't prose (mobile/android's `ChatLine`).
///
/// A glyph (read as `glyphLabel`, or decorative without one), a one-line title in its font, an optional subtitle under
/// it, and an optional status (glyph and word) at the end. With a toggle the whole line is a 44 pt target that opens
/// its details under it, indented to the title, and VoiceOver reads whether it is open and what a tap does.
internal struct ChatLine<Details: View>: View {
    @Environment(\.accessibilityReduceMotion)
    private var reduceMotion

    internal let symbol: String
    internal let glyphLabel: String?
    internal let title: String
    internal var titleFont = Typography.chatCode
    internal var titleColor = Color.primary
    internal var glyphColor = Palette.secondaryText.color
    internal var subtitle: String?
    internal var status: ChatStatusLook?
    internal var toggle: ChatToggle?
    @ViewBuilder internal let details: () -> Details

    internal var body: some View {
        let isExpanded = toggle?.isExpanded == true
        VStack(alignment: .leading, spacing: 0) {
            line(isExpanded: isExpanded)
            if isExpanded {
                VStack(alignment: .leading, spacing: Spacing.small) {
                    details()
                }
                .padding(.leading, ChatStyle.detailsIndent)
                .padding(.bottom, Spacing.small)
                .transition(reduceMotion ? .identity : .opacity)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .animation(Motion.animation(Motion.standard, reduceMotion: reduceMotion), value: isExpanded)
    }

    @ViewBuilder
    private func line(isExpanded: Bool) -> some View {
        let row = HStack(spacing: Spacing.small) {
            Image(systemName: symbol)
                .font(.system(size: ChatStyle.glyphPoints))
                .foregroundStyle(glyphColor)
                .frame(width: ChatStyle.glyphSize, height: ChatStyle.glyphSize)
                .accessibilityLabel(glyphLabel ?? "")
                .accessibilityHidden(glyphLabel == nil)
            VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                Text(title)
                    .font(titleFont)
                    .foregroundStyle(titleColor)
                    .lineLimit(isExpanded ? nil : 1)
                    .truncationMode(.tail)
                if let subtitle {
                    Text(subtitle)
                        .font(.footnote)
                        .foregroundStyle(Palette.secondaryText.color)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if let status {
                HStack(spacing: Spacing.xSmall) {
                    StatusGlyph(status.shape, color: status.color.color)
                    Text(status.word)
                        .font(Typography.statusWord)
                        .foregroundStyle(status.color.color)
                }
            }
        }
        .padding(.horizontal, Spacing.xSmall)
        .padding(.vertical, Spacing.xSmall)
        .frame(minHeight: Spacing.hitTarget)
        .contentShape(Rectangle())
        if let toggle {
            Button(action: toggle.onToggle) {
                row
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .combine)
            .accessibilityValue(isExpanded ? String(localized: "Expanded") : String(localized: "Collapsed"))
            .accessibilityHint(isExpanded ? toggle.hide : toggle.show)
        } else {
            row
                .accessibilityElement(children: .combine)
        }
    }
}
