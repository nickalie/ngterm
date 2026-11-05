/// What one `Pane.frame` call brings: the header, the rows that changed, and the whole selection.
public struct FrameUpdate: Sendable {
    /// The header.
    public let header: TerminalHeader
    /// The rows that changed since the frame asked after, top to bottom.
    public let rows: [TerminalRow]
    /// Every row the selection crosses, changed or not.
    public let selection: [SelectedSpan]

    /// An update of `rows` under `header`, with `selection`.
    public init(header: TerminalHeader, rows: [TerminalRow], selection: [SelectedSpan]) {
        self.header = header
        self.rows = rows
        self.selection = selection
    }
}
