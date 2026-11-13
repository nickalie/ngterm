import SwiftUI

/// The app a machine runs when it isn't ngterm (`word`: "agterm"): the word in secondary ink inside a hairline
/// outline, so it reads as a fact about the machine, not a status or an action.
///
/// It takes no tap. What the app lacks (`note`) is its VoiceOver hint and, with a pointer, its tooltip; the screen
/// that has room says it in full too.
public struct PeerBadge: View {
    private let word: String
    private let note: String?

    /// The body of the view.
    public var body: some View {
        Text(verbatim: word)
            .font(Typography.peerWord)
            .lineLimit(1)
            .fixedSize()
            .padding(.horizontal, Spacing.xSmall + Spacing.xxSmall)
            .padding(.vertical, Spacing.xxSmall)
            .overlay {
                RoundedRectangle(cornerRadius: Radius.small, style: .continuous)
                    .strokeBorder(.tertiary, lineWidth: 1)
            }
            .rowForeground(Palette.secondaryText.color)
            .help(Text(verbatim: note ?? word))
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(Text(verbatim: word))
            .accessibilityHint(Text(verbatim: note ?? ""))
    }

    /// The badge of the app named `word`, which lacks what `note` says.
    public init(_ word: String, note: String? = nil) {
        self.word = word
        self.note = note
    }
}
