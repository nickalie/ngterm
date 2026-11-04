import SwiftUI

/// A row's colored text and glyphs: in their color, or, in a selected row (the background's prominence increased: an
/// iPad's sidebar draws the selection in the tint), in the row's own foreground, which reads on the selection where
/// the colors don't.
///
/// The status keeps its glyph and its word there, so it still isn't said by color alone.
internal struct RowForeground: ViewModifier {
    @Environment(\.backgroundProminence)
    private var prominence

    internal let color: Color

    internal func body(content: Content) -> some View {
        if prominence == .increased {
            content.foregroundStyle(.primary)
        } else {
            content.foregroundStyle(color)
        }
    }
}
