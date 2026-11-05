/// A row the selection crosses, from ``first`` to ``last`` column, both included.
public struct SelectedSpan: Hashable, Sendable {
    /// The row.
    public let row: Int
    /// The first selected column.
    public let first: Int
    /// The last selected column.
    public let last: Int

    /// Row `row` selected from `first` to `last`, both included.
    public init(row: Int, first: Int, last: Int) {
        self.row = row
        self.first = first
        self.last = last
    }
}
