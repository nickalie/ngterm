import NgtermCore
import UIKit

/// The pointer's side of the pane a ``TerminalView`` shows (mobile/android's `TerminalInput`).
///
/// It turns points into the pane's cells and pixels, and makes the mouse, wheel, scroll, fling and pinch calls the
/// gestures ask for. A pane with no terminal now (hidden, between attaches) drops pointer input: a pointer event isn't
/// worth queueing.
@preconcurrency
@MainActor
public final class TerminalPointer {
    /// The text sizes a pinch keeps to, in points: Settings' own range (`AppSettings.textSizes`).
    public static let textSizes: ClosedRange<CGFloat> = 8...32

    private unowned let view: TerminalView
    private let fling = TerminalFling()
    private var rowCarry: CGFloat = 0
    private var columnCarry: CGFloat = 0
    private var hovered: CellPoint?

    /// Whether the program takes pointer events: a click and a drag are then the program's.
    public var tracking: Bool {
        view.pane.mouseTracking()
    }

    /// Whether a fling is still scrolling.
    public var isFlinging: Bool {
        fling.isRunning
    }

    internal init(view: TerminalView) {
        self.view = view
    }

    /// The cell under `point` (the view's points), kept inside the grid; `nil` before the view has a grid.
    public func cell(at point: CGPoint) -> CellPoint? {
        guard let grid = view.grid else {
            return nil
        }
        let font = view.font
        let column = min(max(Int((point.x / font.cellWidth).rounded(.down)), 0), Int(grid.cols) - 1)
        let row = min(max(Int((point.y / font.cellHeight).rounded(.down)), 0), Int(grid.rows) - 1)
        return CellPoint(x: UInt16(column), y: UInt32(row))
    }

    /// Reports a pointer event at `point` to the program; whether it took it (it does only while ``tracking``).
    @discardableResult
    public func mouse(_ action: PaneMouseAction, button: PaneMouseButton?, mods: UInt8, at point: CGPoint) -> Bool {
        let pixels = panePixels(point)
        let event = PaneMouse(action: action, button: button, mods: mods, x: Float(pixels.x), y: Float(pixels.y))
        return (try? view.pane.mouse(mouse: event)) == true
    }

    /// The pointer moved over `point` with no button down: a program tracking every motion hears of each new cell.
    public func hover(at point: CGPoint, mods: UInt8) {
        let cell = cell(at: point)
        guard let cell, cell != hovered else {
            return
        }
        hovered = cell
        mouse(.motion, button: nil, mods: mods, at: point)
    }

    /// Turns the wheel by whole `rows` (up is positive) and `columns` (right) at `point`: the pane routes it.
    public func wheel(rows: Int, columns: Int, mods: UInt8, at point: CGPoint) {
        guard rows != 0 || columns != 0 else {
            return
        }
        let pixels = panePixels(point)
        let turn = PaneWheel(
            rows: Int32(rows),
            columns: Int32(columns),
            mods: mods,
            x: Float(pixels.x),
            y: Float(pixels.y)
        )
        _ = try? view.pane.wheel(wheel: turn)
        view.dirty()
    }

    /// A finger dragged the screen `points` down (negative: up): the rows come down with it, as a wheel turned up.
    public func drag(by points: CGFloat, at point: CGPoint) {
        rowCarry += points
        let rows = whole(&rowCarry, per: view.font.cellHeight)
        wheel(rows: rows, columns: 0, mods: 0, at: point)
    }

    /// A trackpad's or a mouse wheel's scroll moved the content by `delta` points (down and right positive, as the
    /// fingers go): the rows come down with it, and the columns go the other way, as a wheel turned right.
    public func scroll(by delta: CGPoint, mods: UInt8, at point: CGPoint) {
        rowCarry += delta.y
        columnCarry -= delta.x
        let rows = whole(&rowCarry, per: view.font.cellHeight)
        let columns = whole(&columnCarry, per: view.font.cellWidth)
        wheel(rows: rows, columns: columns, mods: mods, at: point)
    }

    /// Goes on scrolling after a drag let go at `velocity` points a second (down positive), slowing as a list does.
    public func fling(velocity: CGFloat, at point: CGPoint) {
        fling.start(velocity: velocity) { [weak self] points in
            guard let self, view.window != nil else {
                return false
            }
            drag(by: points, at: point)
            return true
        }
    }

    /// A touch stops a fling where it is, and starts its own drag from no remainder.
    public func stopFling() {
        fling.stop()
        rowCarry = 0
        columnCarry = 0
    }

    /// A pinch scaled the text by `scale` from `base` points: it changes in whole points, within ``textSizes``, so the
    /// pty is resized once per step.
    public func zoom(base: CGFloat, scale: CGFloat) {
        let size = min(max((base * scale).rounded(), Self.textSizes.lowerBound), Self.textSizes.upperBound)
        if size != view.textSize {
            view.textSize = size
        }
    }

    // the whole steps of `step` points in `carry`, toward zero, leaving the rest in it
    private func whole(_ carry: inout CGFloat, per step: CGFloat) -> Int {
        let steps = (carry / step).rounded(.towardZero)
        carry -= steps * step
        return Int(steps)
    }

    // the view's points as the pane's pixels: it counts cells of the whole pixels it was given, the view draws them
    // unrounded
    private func panePixels(_ point: CGPoint) -> CGPoint {
        guard let grid = view.grid else {
            return point
        }
        let font = view.font
        return CGPoint(
            x: point.x / font.cellWidth * CGFloat(grid.cellWidth),
            y: point.y / font.cellHeight * CGFloat(grid.cellHeight)
        )
    }

    deinit {
        // the fling holds only a weak reference back
    }
}
