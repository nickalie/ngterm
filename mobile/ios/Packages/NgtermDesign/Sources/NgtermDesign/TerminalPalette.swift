/// A terminal's colors: the ground, the text, the cursor, the selection and the 16 ANSI colors (0-7, then bright).
///
/// The palettes are the Android app's (`ui/theme/TerminalPalette.kt`), value for value: `TerminalPaletteTests` reads
/// that file and holds the two equal.
public struct TerminalPalette: Hashable, Sendable, Identifiable {
    /// How many ANSI colors a palette sets; the 256-color cube and ramp above them are the terminal's own.
    public static let ansiCount = 16

    /// The dark default: a graphite ground, with a copper cursor and verdigris for cyan.
    public static let copper = Self(
        id: "copper",
        background: "#1b2226",
        foreground: "#e3e8ea",
        cursor: "#e3884f",
        selectionBackground: "#3d4e57",
        ansi: [
            "#2a343a", "#e5675c", "#8cc084", "#e3b65a", "#6fa3c8", "#c08bb5", "#5fbfaa", "#c9d1d4",
            "#6f7c82", "#f2867a", "#a6d69b", "#f0cb78", "#8dbce0", "#d6a5cb", "#84d6c4", "#f2f5f6",
        ]
    )

    /// The light default: the drafting ground with ink text; the ANSI colors are darkened to read on it.
    public static let drafting = Self(
        id: "drafting",
        background: "#f6f8f8",
        foreground: "#1b2226",
        cursor: "#984a19",
        selectionBackground: "#ffdcc8",
        ansi: [
            "#1b2226", "#b3362a", "#3d7a32", "#8a6410", "#2f64a0", "#8a4a85", "#1a6a5d", "#8b979c",
            "#5b676d", "#c8463a", "#4f8e43", "#a07818", "#3d78b8", "#a05c9a", "#237f70", "#a9b4b9",
        ]
    )

    /// Ghostty's defaults, as desktop ngterm starts with them.
    ///
    /// Its `#282c34` ground and white text, and libghostty-vt's built-in 16 colors.
    public static let ghostty = Self(
        id: "ghostty",
        background: "#282c34",
        foreground: "#ffffff",
        cursor: "#ffffff",
        selectionBackground: "#3e4451",
        ansi: [
            "#1d1f21", "#cc6666", "#b5bd68", "#f0c674", "#81a2be", "#b294bb", "#8abeb7", "#c5c8c6",
            "#666666", "#d54e53", "#b9ca4a", "#e7c547", "#7aa6da", "#c397d8", "#70c0b1", "#eaeaea",
        ]
    )

    /// The palettes the app ships, the dark default first.
    public static let builtIn: [Self] = [.copper, .drafting, .ghostty]

    /// The palette's name in settings and in the stores.
    public let id: String
    /// The ground.
    public let background: RGB
    /// The text.
    public let foreground: RGB
    /// The cursor.
    public let cursor: RGB
    /// The selection's fill, under text that keeps its own colors.
    public let selectionBackground: RGB
    /// The 16 ANSI colors.
    public let ansi: [RGB]

    private init(id: String, background: RGB, foreground: RGB, cursor: RGB, selectionBackground: RGB, ansi: [RGB]) {
        precondition(ansi.count == Self.ansiCount, "a palette has \(Self.ansiCount) ANSI colors, not \(ansi.count)")
        self.id = id
        self.background = background
        self.foreground = foreground
        self.cursor = cursor
        self.selectionBackground = selectionBackground
        self.ansi = ansi
    }

    /// The palette the theme picks when settings pin none: Copper on the dark side, Drafting on the light.
    public static func matching(isDark: Bool) -> Self {
        isDark ? .copper : .drafting
    }
}
