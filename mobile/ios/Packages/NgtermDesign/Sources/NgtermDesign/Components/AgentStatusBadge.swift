import SwiftUI

/// An agent's status as its origin sets it.
///
/// The word for `status` (the origin's wire name), and the glyph and color ``AgentStatusLook`` resolves, the color
/// lifted until it reads on every system ground of the environment's appearance. A `label` says more than the word
/// where there is more to say ("2 waiting").
public struct AgentStatusBadge: View {
    @Environment(\.colorScheme)
    private var colorScheme
    @Environment(\.colorSchemeContrast)
    private var contrast

    private let status: String?
    private let colorHex: String?
    private let shape: String?
    private let label: String?

    /// The body of the view.
    public var body: some View {
        let look = AgentStatusLook(
            status: status,
            colorHex: colorHex,
            shape: shape,
            in: Appearance(colorScheme: colorScheme, contrast: contrast)
        )
        GlyphAndWord(shape: look.shape, color: look.color.color, word: label ?? look.word)
    }

    /// The badge of `status`, with the `#rrggbb` color and the shape its origin sets, if any, and `label` in place of
    /// its word.
    public init(status: String?, colorHex: String? = nil, shape: String? = nil, label: String? = nil) {
        self.status = status
        self.colorHex = colorHex
        self.shape = shape
        self.label = label
    }
}
