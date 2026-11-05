/// One row a frame brought: its index in the grid, whether its text continues on the next row, and its runs.
///
/// A class, so a row sent again is a new record: what was laid out from the old one (``GlyphCache``) is told apart by
/// identity.
public final class TerminalRow: Sendable {
    /// The row's index in the grid, from the top.
    public let index: Int
    /// Whether its text continues on the next row (a soft wrap).
    public let wrapped: Bool
    /// Its runs, left to right; the cells after the last one are blank.
    public let runs: [TerminalRun]

    /// Row `index` with `runs`, `wrapped` when its text continues on the next row.
    public init(index: Int, wrapped: Bool, runs: [TerminalRun]) {
        self.index = index
        self.wrapped = wrapped
        self.runs = runs
    }

    /// The run covering `column`, if any.
    public func run(at column: Int) -> TerminalRun? {
        runs.first { $0.covers(column) }
    }

    deinit {
        // nothing to release
    }
}
