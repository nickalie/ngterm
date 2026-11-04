import SwiftUI

/// One of the app's statuses: its glyph and its word, both in its color, read by VoiceOver as the word.
///
/// A `label` says more than the word where the state has more to say ("Retrying in 4 s").
public struct StatusBadge: View {
    private let tone: StatusTone
    private let label: String?

    /// The body of the view.
    public var body: some View {
        GlyphAndWord(shape: tone.shape, color: tone.color.color, word: label ?? tone.word)
    }

    /// The badge of `tone`, with `label` in place of its word.
    public init(_ tone: StatusTone, label: String? = nil) {
        self.tone = tone
        self.label = label
    }
}
