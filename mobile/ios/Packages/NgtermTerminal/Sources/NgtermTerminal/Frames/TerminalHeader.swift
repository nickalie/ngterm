/// A frame's header: its seq, its grid, the default colors and the cursor.
public struct TerminalHeader: Hashable, Sendable {
    /// The frame's seq: pass it to the next `frame` call.
    public let seq: UInt64
    /// Whether every row follows: draw from scratch.
    public let full: Bool
    /// Whether the terminal was busy and this is the frame made last: ask again soon.
    public let stale: Bool
    /// The grid's columns.
    public let columns: Int
    /// The grid's rows.
    public let rows: Int
    /// The default text color, `0xAARRGGBB`.
    public let defaultForeground: UInt32
    /// The default ground color, `0xAARRGGBB`.
    public let defaultBackground: UInt32
    /// The cursor's color; `0`: none set, the cursor follows the cell's text.
    public let cursorColor: UInt32
    /// The cursor.
    public let cursor: TerminalCursor

    /// A header with these fields.
    public init(
        seq: UInt64,
        full: Bool,
        stale: Bool,
        grid: (columns: Int, rows: Int),
        defaults: (foreground: UInt32, background: UInt32),
        cursorColor: UInt32,
        cursor: TerminalCursor
    ) {
        self.seq = seq
        self.full = full
        self.stale = stale
        columns = grid.columns
        rows = grid.rows
        defaultForeground = defaults.foreground
        defaultBackground = defaults.background
        self.cursorColor = cursorColor
        self.cursor = cursor
    }
}
