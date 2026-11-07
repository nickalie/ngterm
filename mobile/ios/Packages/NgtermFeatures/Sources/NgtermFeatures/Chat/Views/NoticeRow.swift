import NgtermCore
import NgtermDesign
import SwiftUI

/// Something that happened to the conversation (mobile/android's `NoticeRow`), as a quiet line with its glyph: an
/// interrupt, a compaction (a tap shows its summary when the transcript has it), an API error in the failed color, a
/// summary.
internal struct NoticeRow: View {
    internal let kind: ChatNoticeKind
    internal let text: String
    internal let detail: String?
    internal let isOpen: Bool
    internal let onToggle: @MainActor () -> Void

    internal var body: some View {
        let color = kind == .apiError ? Palette.failed.color : Palette.secondaryText.color
        let opens = detail.map { !AskText.isBlank($0) } ?? false
        ChatLine(
            symbol: ChatStyle.symbol(kind),
            glyphLabel: ChatStyle.word(kind),
            title: AskText.isBlank(text) ? ChatStyle.word(kind) : text,
            titleFont: .subheadline,
            titleColor: color,
            glyphColor: color,
            toggle: opens
                ? ChatToggle(
                    isExpanded: isOpen,
                    show: String(localized: "Shows the summary"),
                    hide: String(localized: "Hides the summary"),
                    onToggle: onToggle
                )
                : nil
        ) {
            if let detail {
                Text(detail)
                    .font(.subheadline)
                    .foregroundStyle(Palette.secondaryText.color)
                    .textSelection(.enabled)
            }
        }
    }
}
