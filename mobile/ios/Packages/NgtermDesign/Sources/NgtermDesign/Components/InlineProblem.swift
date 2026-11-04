import SwiftUI

/// What is wrong with a field or a step, said where it happened: the failed glyph and the fix, in the failed color.
///
/// VoiceOver reads it as one element, its words.
public struct InlineProblem: View {
    private let message: String

    /// The body of the view.
    public var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.xSmall) {
            StatusGlyph(.triangle, color: Palette.failed.color)
            Text(message)
                .font(Typography.note)
                .foregroundStyle(Palette.failed.color)
                .fixedSize(horizontal: false, vertical: true)
        }
        .accessibilityElement(children: .combine)
    }

    /// The problem `message`.
    public init(_ message: String) {
        self.message = message
    }
}
