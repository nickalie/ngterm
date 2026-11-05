/// A row's text laid out on the grid: what to draw for each run, shaped once per version of the row.
public struct RowLayout {
    /// The row record it was laid out from: a row sent again is a new record, laid out anew.
    public let source: TerminalRow
    /// The draws, left to right.
    public let draws: [GlyphDraw]
}
