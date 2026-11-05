import NgtermDesign
import NgtermPlatform

/// How a session's terminals draw, from Settings: the palette (the theme's follows the appearance), the text size and
/// the font.
internal struct TerminalLook: Equatable {
    /// The palette.
    internal let palette: TerminalPalette
    /// The text size, in points.
    internal let textSize: Double
    /// The font's faces: `nil` is the system's monospace.
    internal let faces: TerminalFaces?

    /// The look `settings` give in the dark appearance when `isDark`, the light one otherwise.
    internal init(_ settings: AppSettings, isDark: Bool) {
        switch settings.palette {
        case .copper:
            palette = .copper

        case .drafting:
            palette = .drafting

        case .ghostty:
            palette = .ghostty

        case .theme:
            palette = .matching(isDark: isDark)
        }
        textSize = Double(settings.textSize)
        faces = settings.font == .jetBrainsMono ? Fonts.terminalFaces : nil
    }
}
