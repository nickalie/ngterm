import SwiftUI

/// What is under way in place of a status: a small spinner and the word for it ("Ending", "Duplicating"), read by
/// VoiceOver as the word.
public struct UnderWayLabel: View {
    private let word: String

    /// The body of the view.
    public var body: some View {
        HStack(spacing: Spacing.xSmall) {
            ProgressView()
                .controlSize(.mini)
            Text(word)
                .font(Typography.statusWord)
                .rowForeground(Palette.secondaryText.color)
        }
        .accessibilityElement(children: .combine)
    }

    /// The spinner with `word`, what is under way.
    public init(_ word: String) {
        self.word = word
    }
}
