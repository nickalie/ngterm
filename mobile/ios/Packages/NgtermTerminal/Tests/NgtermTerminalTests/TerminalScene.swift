import NgtermCore
import NgtermCoreFakes

/// What a known screen is drawn at: the grid and the colors the view gave its pane.
///
/// The colors are packed as the core packs them: the palette's, resolved as the Rust terminal resolves ANSI colors
/// before packing.
internal struct TerminalScene {
    /// The grid's columns.
    internal let columns: Int
    /// The grid's rows.
    internal let rows: Int
    private let colors: PaneColors?
    private let slots: [UInt32]

    /// A scene `columns` × `rows` in `colors`; `ghosttyANSI` are the terminal's own ANSI colors, before the view gives
    /// the pane a palette.
    internal init(columns: Int, rows: Int, colors: PaneColors?, ghosttyANSI: [UInt32]) {
        self.columns = columns
        self.rows = rows
        self.colors = colors
        slots = colors?.palette ?? ghosttyANSI
    }

    /// ANSI color `slot`, set on the cell.
    internal func ansi(_ slot: Int) -> UInt32 {
        slots[slot]
    }

    /// A writer of this grid in these colors, with `cursor`.
    internal func writer(cursor: FakeCursor) -> RowBufferWriter {
        RowBufferWriter(columns: columns, rows: rows, cursor: cursor)
            .colors(
                foreground: colors?.foreground ?? RowBufferWriter.ghosttyForeground,
                background: colors?.background ?? RowBufferWriter.ghosttyBackground,
                cursor: colors?.cursor ?? 0
            )
    }
}
