import CoreGraphics
import NgtermCore

/// Where a touch selection's two handles stand, in the view's points (mobile/android's `HandleSpots`, with iOS's
/// handles).
public struct HandleSpots: Equatable, Sendable {
    /// A handle's knob: its radius, in points.
    public static let knobRadius: CGFloat = 5
    /// How near a handle a finger lands to take it, in points: half a 44 pt hit target.
    public static let reach: CGFloat = 22
    private static let halves: CGFloat = 2

    /// The selection's start.
    public let start: SelectionHandle
    /// The selection's end.
    public let end: SelectionHandle
    /// The selection's rows, from the top of the first to the bottom of the last, across the view: the edit menu
    /// points at it.
    public let rows: CGRect

    /// The handles of `selection` (a frame's spans, whole) in the cells of `font`, in a view `size` big; `nil` with no
    /// selection.
    public init?(_ selection: [SelectedSpan], font: TerminalFont, size: CGSize) {
        guard let first = selection.min(by: { $0.row < $1.row }), let last = selection.max(by: { $0.row < $1.row })
        else {
            return nil
        }
        let startCell = CellPoint(x: UInt16(first.first), y: UInt32(first.row))
        let endCell = CellPoint(x: UInt16(last.last), y: UInt32(last.row))
        let radius = Self.knobRadius
        let startX = font.left(first.first)
        let startTop = font.top(first.row)
        let startBottom = startTop + font.cellHeight
        let endX = font.left(last.last + 1)
        let endTop = font.top(last.row)
        let endBottom = endTop + font.cellHeight
        // the start's knob above its stem and the end's below, unless that leaves the view
        let diameter = radius * Self.halves
        let startKnob = startTop - diameter >= 0 ? startTop - radius : startBottom + radius
        let endKnob = endBottom + diameter <= size.height ? endBottom + radius : endTop - radius
        start = SelectionHandle(
            cell: startCell,
            fixed: endCell,
            cellCenter: Self.center(first.first, first.row, font),
            stemTop: CGPoint(x: startX, y: startTop),
            stemBottom: CGPoint(x: startX, y: startBottom),
            knob: CGPoint(x: startX, y: startKnob)
        )
        end = SelectionHandle(
            cell: endCell,
            fixed: startCell,
            cellCenter: Self.center(last.last, last.row, font),
            stemTop: CGPoint(x: endX, y: endTop),
            stemBottom: CGPoint(x: endX, y: endBottom),
            knob: CGPoint(x: endX, y: endKnob)
        )
        rows = CGRect(x: 0, y: startTop, width: size.width, height: endBottom - startTop)
    }

    private static func center(_ column: Int, _ row: Int, _ font: TerminalFont) -> CGPoint {
        CGPoint(
            x: font.left(column) + font.cellWidth / Self.halves,
            y: font.top(row) + font.cellHeight / Self.halves
        )
    }

    /// The handle within ``reach`` of `point`, the nearer of the two; `nil` for neither.
    public func nearest(to point: CGPoint) -> SelectionHandle? {
        [start, end]
            .map { ($0, $0.distance(to: point)) }
            .filter { $0.1 <= Self.reach }
            .min { $0.1 < $1.1 }?
            .0
    }
}
