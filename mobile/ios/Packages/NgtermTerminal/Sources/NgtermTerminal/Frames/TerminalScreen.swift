import Foundation

/// The screen a ``TerminalView`` draws: the rows it holds from the frames read so far, each replaced as a frame brings
/// it again, with the latest frame's header and selection (mobile/android's `TerminalScreen`).
///
/// Taking a frame says which rows look different now, so the view redraws only those: the rows the frame brought, the
/// rows whose selection changed, the cursor's old and new rows, and every row when the grid or the default colors
/// changed (a run's default colors are resolved at draw time, so a row the reader holds recolors without being sent).
public struct TerminalScreen: Sendable {
    /// The latest frame's header; `nil` before the first.
    public private(set) var header: TerminalHeader?
    /// The selection's rows, whole, as the latest frame listed them.
    public private(set) var selection: [SelectedSpan] = []
    private var rows: [TerminalRow?] = []
    private var selected: [Int: SelectedSpan] = [:]

    /// The seq to ask the next frame since: the latest frame's, or 0 before the first.
    public var seq: UInt64 {
        header?.seq ?? 0
    }

    /// The grid's columns, 0 before the first frame.
    public var columns: Int {
        header?.columns ?? 0
    }

    /// The grid's rows, 0 before the first frame.
    public var rowCount: Int {
        rows.count
    }

    /// An empty screen, before its first frame.
    public init() {
        // nothing read yet
    }

    private static func recolored(_ previous: TerminalHeader, _ next: TerminalHeader) -> Bool {
        previous.defaultForeground != next.defaultForeground || previous.defaultBackground != next.defaultBackground
    }

    /// The row at `index` as last sent; `nil` for a row with nothing drawn on it (not sent since a full frame).
    public func row(_ index: Int) -> TerminalRow? {
        rows.indices.contains(index) ? rows[index] : nil
    }

    /// The selected span of row `index`, if the selection crosses it.
    public func selection(of index: Int) -> SelectedSpan? {
        selected[index]
    }

    /// Takes `update` and says which rows to draw again.
    ///
    /// A full frame, or one whose grid differs, starts from blank rows; the rows it brings replace the ones held, and a
    /// row past the grid is dropped.
    public mutating func apply(_ update: FrameUpdate) -> IndexSet {
        let next = update.header
        let previous = header
        let regrid = next.full || next.rows != rows.count || next.columns != previous?.columns
        if regrid {
            rows = Array(repeating: nil, count: next.rows)
        }
        var changed = IndexSet()
        for row in update.rows where rows.indices.contains(row.index) {
            rows[row.index] = row
            changed.insert(row.index)
        }
        let nextSelected = Dictionary(update.selection.map { ($0.row, $0) }) { first, _ in first }
        if let previous, !regrid, !Self.recolored(previous, next) {
            for index in Set(selected.keys).union(nextSelected.keys) where selected[index] != nextSelected[index] {
                changed.insert(index)
            }
            if previous.cursor != next.cursor || previous.cursorColor != next.cursorColor {
                changed.insert(previous.cursor.row)
                changed.insert(next.cursor.row)
            }
        } else {
            changed = IndexSet(integersIn: rows.indices)
        }
        header = next
        selection = update.selection
        selected = nextSelected
        return changed.filteredIndexSet { rows.indices.contains($0) }
    }

    /// The text of row `index` as drawn, its blank cells spaces, trailing blanks trimmed (for tests and accessibility).
    public func text(_ index: Int) -> String {
        guard let row = row(index) else {
            return ""
        }
        var line = ""
        var column = 0
        for run in row.runs {
            line += String(repeating: " ", count: max(run.column - column, 0))
            line += run.text
            column = run.end
        }
        while line.last == " " {
            line.removeLast()
        }
        return line
    }
}
