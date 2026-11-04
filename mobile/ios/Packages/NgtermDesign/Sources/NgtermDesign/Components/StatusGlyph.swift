import SwiftUI

/// A status's glyph alone, in `color`, sized with the text around it.
///
/// Decorative: the word next to it is what VoiceOver reads.
public struct StatusGlyph: View {
    private let shape: StatusShape
    private let color: Color

    /// The body of the view.
    public var body: some View {
        Image(systemName: shape.symbolName)
            .font(Typography.statusWord)
            .imageScale(.small)
            .rowForeground(color)
            .accessibilityHidden(true)
    }

    /// The glyph of `shape` in `color`.
    public init(_ shape: StatusShape, color: Color) {
        self.shape = shape
        self.color = color
    }
}
