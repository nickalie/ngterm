import NgtermCore
import NgtermCoreFakes
import NgtermTerminal
import UIKit

/// A terminal view over a ``FakePane`` that records the input, for the gestures' tests (mobile/android's
/// `SelectingPane`): its frames show ``text`` and the selection a test sets with ``show(_:)``, as the core's would
/// after a select call.
@MainActor
internal final class TerminalRig {
    /// The text size it draws at.
    internal static let textSize: CGFloat = 13
    /// The rows of text its frames show.
    nonisolated internal static let text = ["hello world", "second line", "", "the end"]
    nonisolated private static let columns = 40
    nonisolated private static let rows = 10
    private static let width: CGFloat = 390
    private static let height: CGFloat = 200
    private static let scale: CGFloat = 2
    private static let halves: CGFloat = 2

    /// The pane, which records the input.
    internal let pane: FakePane
    /// The clipboard, in memory.
    internal let clipboard: MemoryClipboard
    /// The view.
    internal let view: TerminalView
    private let spans: Locked<[SelectedSpan]>

    /// The input the pane took, in order.
    internal var inputs: [FakeInput] {
        pane.inputs
    }

    /// A rig whose clipboard holds `clipboard`, 390 × 200 points at a display scale of 2, with its first frame read.
    internal init(clipboard held: String?) {
        let shown = Locked<[SelectedSpan]>([])
        spans = shown
        pane = FakePane { since in
            var writer = RowBufferWriter(columns: Self.columns, rows: Self.rows, seq: since + 1)
            for (row, line) in Self.text.enumerated() where !line.isEmpty {
                writer = writer.text(row, line)
            }
            for span in shown.value {
                writer = writer.select(span.row, first: span.first, last: span.last)
            }
            return writer.bytes()
        }
        clipboard = MemoryClipboard(held)
        view = TerminalView(pane: pane, palette: .copper, textSize: Self.textSize, clipboard: clipboard)
        view.traitOverrides.displayScale = Self.scale
        view.updateTraitsIfNeeded()
        view.frame = CGRect(x: 0, y: 0, width: Self.width, height: Self.height)
        view.layoutIfNeeded()
        view.drawFrame()
    }

    /// Cell `column`, `row`.
    internal static func cell(_ column: Int, _ row: Int) -> CellPoint {
        CellPoint(x: UInt16(column), y: UInt32(row))
    }

    /// The middle of cell `column`, `row`, in the view's points.
    internal func point(_ column: Int, _ row: Int) -> CGPoint {
        let font = view.font
        return CGPoint(
            x: font.left(column) + font.cellWidth / Self.halves,
            y: font.top(row) + font.cellHeight / Self.halves
        )
    }

    /// The pane's pixels at `point`, as a pointer event carries them.
    internal func pixels(_ point: CGPoint) -> (x: Float, y: Float) {
        let font = view.font
        let grid = view.grid ?? PaneSize(cols: 1, rows: 1, cellWidth: 1, cellHeight: 1)
        return (
            Float(point.x / font.cellWidth * CGFloat(grid.cellWidth)),
            Float(point.y / font.cellHeight * CGFloat(grid.cellHeight))
        )
    }

    /// The next frame shows `selection` selected, and the view reads it.
    internal func show(_ selection: [SelectedSpan]) {
        spans.value = selection
        view.dirty()
        view.drawFrame()
    }

    /// The input taken since `count` inputs.
    internal func inputs(after count: Int) -> [FakeInput] {
        Array(pane.inputs.dropFirst(count))
    }

    deinit {
        // nothing to release
    }
}
