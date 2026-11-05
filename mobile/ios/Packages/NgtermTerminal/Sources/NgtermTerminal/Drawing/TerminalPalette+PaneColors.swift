import NgtermCore
import NgtermDesign

extension TerminalPalette {
    /// The palette as the pane's colors: the ground, the text, the cursor and the 16 ANSI colors; the selection is the
    /// view's to draw.
    public var paneColors: PaneColors {
        PaneColors(
            foreground: foreground.argb,
            background: background.argb,
            cursor: cursor.argb,
            palette: ansi.map(\.argb)
        )
    }
}
