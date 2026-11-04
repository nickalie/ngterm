import SwiftUI

/// A status's glyph and word in its color, read by VoiceOver as the word: what every badge draws.
internal struct GlyphAndWord: View {
    internal let shape: StatusShape
    internal let color: Color
    internal let word: String

    internal var body: some View {
        HStack(spacing: Spacing.xSmall) {
            StatusGlyph(shape, color: color)
            Text(word)
                .font(Typography.statusWord)
                .rowForeground(color)
                .fixedSize(horizontal: false, vertical: true)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(word)
    }
}
