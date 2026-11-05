/// The measures of a session's split (``SplitLayout``).
internal enum SplitMetrics {
    /// How many panes a split shows.
    internal static let panes = 2
    /// The origin's axis of two panes side by side: the divider runs vertically.
    internal static let sideBySide = "vertical"
    /// The focus bar's thickness: as thick as the gutter it sits in leaves room for.
    internal static let barWidth: Double = 3
    /// The focus bar's length: about two rows of text.
    internal static let barLength: Double = 32
}
