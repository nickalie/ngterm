/// The terminal's palette: the theme's (Copper when dark, Drafting when light), or one pinned whatever the theme.
public enum PaletteChoice: StoredChoice {
    /// Copper, whatever the theme.
    case copper
    /// Drafting, whatever the theme.
    case drafting
    /// Ghostty's defaults, whatever the theme.
    case ghostty
    /// The theme's: Copper when dark, Drafting when light.
    case theme
}
