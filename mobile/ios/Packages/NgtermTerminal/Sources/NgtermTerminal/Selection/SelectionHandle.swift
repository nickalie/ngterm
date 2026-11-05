import CoreGraphics
import NgtermCore

/// One end of a touch selection, as a finger on its handle moves it.
///
/// A handle is a stem on the selection's edge, as tall as its row, with a round knob: the start's above its stem, the
/// end's below it, as iOS draws its text selection (or on the other side where the view has no room for it).
public struct SelectionHandle: Equatable, Sendable {
    /// The end's cell.
    public let cell: CellPoint
    /// The other end's cell, which stays put while this one moves.
    public let fixed: CellPoint
    /// The end cell's middle, in the view's points: a finger on the handle moves this point.
    public let cellCenter: CGPoint
    /// The stem's top, on the selection's edge.
    public let stemTop: CGPoint
    /// The stem's bottom.
    public let stemBottom: CGPoint
    /// The knob's middle.
    public let knob: CGPoint

    /// How far `point` is from the handle (its knob and its stem), in points.
    public func distance(to point: CGPoint) -> CGFloat {
        let top = min(stemTop.y, knob.y)
        let bottom = max(stemBottom.y, knob.y)
        let along = min(max(point.y, top), bottom)
        return hypot(point.x - stemTop.x, point.y - along)
    }
}
