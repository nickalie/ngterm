import NgtermDesign
import SwiftUI

/// A palette's ground with its eight normal ANSI colors on it: a strip beside a palette's choice.
///
/// VoiceOver reads the choice's words instead.
internal struct PaletteSwatch: View {
    private static let normalColors = 8
    private static let width: Double = 88
    private static let height: Double = 20

    internal let palette: TerminalPalette

    internal var body: some View {
        HStack(spacing: 0) {
            ForEach(0..<Self.normalColors, id: \.self) { index in
                palette.ansi[index].color
            }
        }
        .padding(Spacing.xSmall)
        .frame(width: Self.width, height: Self.height)
        .background(palette.background.color, in: RoundedRectangle(cornerRadius: Spacing.xSmall, style: .continuous))
        .accessibilityHidden(true)
    }
}
