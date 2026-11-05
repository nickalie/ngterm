import CoreGraphics
import NgtermCore

/// How a finger or the pointer holds the selection while it moves (mobile/android's `Grip` and `WordGrip`).
///
/// A plain grip moves one end from the cell under the pointer plus ``offset`` (a handle's knob sits off its cell), the
/// other end ``fixed``. A word grip holds the word a long press or a double tap selected at its anchor, as text
/// selection does: a drag past the word's end selects from its start, a drag before its start from its end, and within
/// it the word stays. The word is what a frame shows selected on the anchor's row around it; until a frame has it, the
/// anchor stands for it.
internal struct SelectionGrip {
    // a word's first and last cells
    private typealias Word = (start: CellPoint, end: CellPoint)

    /// The end that stays put (a plain grip's).
    internal let fixed: CellPoint
    /// What to add to the pointer to find the cell it moves.
    internal let offset: CGPoint
    private let anchor: CellPoint?
    private var word: Word?

    /// The cell it last moved to.
    internal private(set) var moving: CellPoint

    private init(fixed: CellPoint, moving: CellPoint, offset: CGPoint, anchor: CellPoint?) {
        self.fixed = fixed
        self.moving = moving
        self.offset = offset
        self.anchor = anchor
    }

    /// A grip moving `moving` with `fixed` held, from the pointer plus `offset`.
    internal static func plain(fixed: CellPoint, moving: CellPoint, offset: CGPoint) -> Self {
        Self(fixed: fixed, moving: moving, offset: offset, anchor: nil)
    }

    /// A grip on the word selected at `anchor`.
    internal static func word(at anchor: CellPoint) -> Self {
        Self(fixed: anchor, moving: anchor, offset: .zero, anchor: anchor)
    }

    private static func before(_ first: CellPoint, _ second: CellPoint) -> Bool {
        first.y < second.y || (first.y == second.y && first.x < second.x)
    }

    private static func shownWord(at anchor: CellPoint, spans: [SelectedSpan]) -> Word? {
        let column = Int(anchor.x)
        guard spans.count == 1, let span = spans.first, span.row == Int(anchor.y), span.first...span.last ~= column
        else {
            return nil
        }
        return (CellPoint(x: UInt16(span.first), y: anchor.y), CellPoint(x: UInt16(span.last), y: anchor.y))
    }

    /// The pointer is over `cell` (with the offset added): selects to it in `selection` unless it is the cell already
    /// moved to. `spans` are the selection's rows as the latest frame shows them.
    @MainActor
    internal mutating func move(to cell: CellPoint, in selection: TerminalSelection, spans: [SelectedSpan]) {
        guard cell != moving else {
            return
        }
        moving = cell
        guard let anchor else {
            selection.select(from: fixed, to: cell)
            return
        }
        let held = word ?? Self.shownWord(at: anchor, spans: spans)
        word = held
        let (start, end) = held ?? (anchor, anchor)
        if Self.before(cell, start) {
            selection.select(from: end, to: cell)
        } else if Self.before(end, cell) {
            selection.select(from: start, to: cell)
        } else {
            selection.select(from: start, to: end)
        }
    }
}
