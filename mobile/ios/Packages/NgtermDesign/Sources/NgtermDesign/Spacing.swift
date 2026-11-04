/// The spacing grid and the sizes (DESIGN.md > Spacing, size and shape), in points.
public enum Spacing {
    /// 2 pt: between a title and its note.
    public static let xxSmall: Double = 2
    /// 4 pt: between a glyph and its word, between a row's lines.
    public static let xSmall: Double = 4
    /// 8 pt: between a row's parts.
    public static let small: Double = 8
    /// 12 pt: a banner's padding, between a panel's parts.
    public static let medium: Double = 12
    /// 16 pt: a phone's gutter.
    public static let large: Double = 16
    /// 24 pt: a tablet's gutter, a companion's indent, a cover's padding.
    public static let xLarge: Double = 24
    /// 32 pt.
    public static let xxLarge: Double = 32
    /// 48 pt.
    public static let xxxLarge: Double = 48

    /// The least a tappable thing measures on its short side (HIG).
    public static let hitTarget: Double = 44
    /// Lists and forms stop growing here, centered beyond it.
    public static let readableWidth: Double = 640
    /// A text block stops growing here, under 80 characters.
    public static let textWidth: Double = 420
    /// The most lines a row's name or line wraps to before it ends in an ellipsis.
    public static let rowLines = 2
}
