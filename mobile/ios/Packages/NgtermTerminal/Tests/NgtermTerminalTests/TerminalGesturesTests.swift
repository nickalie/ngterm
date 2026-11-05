import CoreGraphics
import NgtermCore
import NgtermCoreFakes
import NgtermTerminal
import Testing

/// A finger's gestures against the fake pane, which records the input (mobile/android's `TerminalGesturesTest`).
@MainActor
@Suite("a finger's gestures")
internal struct TerminalGesturesTests {
    // the wheel turns a fake pane took, as rows and columns
    private static func turns(_ inputs: [FakeInput]) -> [(rows: Int32, columns: Int32)] {
        inputs.compactMap { input in
            if case .wheel(let wheel) = input {
                return (wheel.rows, wheel.columns)
            }
            return nil
        }
    }

    @Test("a drag scrolls by the whole rows it covers, the rows coming down with the finger")
    internal func dragScrolls() {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.gestures
        let row = rig.view.font.cellHeight
        let start = rig.point(5, 5)
        let down = CGPoint(x: start.x, y: start.y + 2.5 * row)
        gestures.pan(.began, at: down, translation: CGPoint(x: 0, y: 2.5 * row), velocity: .zero)
        let back = CGPoint(x: start.x, y: start.y - row)
        gestures.pan(.changed, at: back, translation: CGPoint(x: 0, y: -row), velocity: .zero)
        gestures.pan(.ended, at: back, translation: CGPoint(x: 0, y: -row), velocity: .zero)
        let turns = Self.turns(rig.inputs)
        #expect(turns.map(\.rows) == [2, -3])
        #expect(turns.allSatisfy { $0.columns == 0 })
        #expect(!rig.view.pointer.isFlinging)
    }

    @Test("a flick goes on scrolling after the finger leaves, and a touch stops it; a cancelled drag doesn't fling")
    internal func flick() {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.gestures
        let point = rig.point(5, 5)
        gestures.pan(.began, at: point, translation: .zero, velocity: .zero)
        gestures.pan(.ended, at: point, translation: .zero, velocity: CGPoint(x: 0, y: 2_000))
        #expect(rig.view.pointer.isFlinging)
        gestures.pan(.began, at: point, translation: .zero, velocity: .zero)
        #expect(!rig.view.pointer.isFlinging)
        gestures.pan(.cancelled, at: point, translation: .zero, velocity: CGPoint(x: 0, y: 2_000))
        #expect(!rig.view.pointer.isFlinging)
        // a finger coming down stops it too, before it is a tap, a drag or a long press
        gestures.pan(.began, at: point, translation: .zero, velocity: .zero)
        gestures.pan(.ended, at: point, translation: .zero, velocity: CGPoint(x: 0, y: 2_000))
        #expect(rig.view.pointer.isFlinging)
        gestures.touchDown()
        #expect(!rig.view.pointer.isFlinging)
        // a slow let-go doesn't fling either
        gestures.pan(.began, at: point, translation: .zero, velocity: .zero)
        gestures.pan(.ended, at: point, translation: .zero, velocity: CGPoint(x: 0, y: 20))
        #expect(!rig.view.pointer.isFlinging)
    }

    @Test("a tap clicks only for a program tracking the mouse")
    internal func tapClicks() {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.gestures
        let point = rig.point(3, 1)
        gestures.tap(at: point, time: 10)
        #expect(rig.inputs.isEmpty)
        rig.pane.tracking = true
        gestures.tap(at: point, time: 20)
        let pixels = rig.pixels(point)
        #expect(
            rig.inputs == [
                .mouse(PaneMouse(action: .press, button: .left, mods: 0, x: pixels.x, y: pixels.y)),
                .mouse(PaneMouse(action: .release, button: .left, mods: 0, x: pixels.x, y: pixels.y)),
            ]
        )
    }

    @Test("a double tap selects the word and a triple tap the line, with handles and the edit menu")
    internal func doubleAndTripleTap() {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.gestures
        let point = rig.point(2, 0)
        gestures.tap(at: point, time: 10)
        gestures.tap(at: CGPoint(x: point.x + 2, y: point.y + 2), time: 10.2)
        #expect(rig.inputs == [.selectWord(TerminalRig.cell(2, 0))])
        #expect(rig.view.selection.isTouchSelection)
        rig.show([SelectedSpan(row: 0, first: 0, last: 4)])
        #expect(rig.view.handleSpots != nil)
        #expect(rig.view.isMenuShown)
        gestures.tap(at: point, time: 10.4)
        #expect(rig.inputs(after: 1) == [.selectLine(TerminalRig.cell(2, 0))])
        // a tap long after is a tap again: it lets the touch selection go
        gestures.tap(at: point, time: 20)
        #expect(rig.inputs(after: 2) == [.clearSelection])
        #expect(!rig.view.selection.isTouchSelection)
        rig.show([])
        #expect(rig.view.handleSpots == nil)
        #expect(!rig.view.isMenuShown)
    }

    @Test("taps apart, in time or on the screen, are single taps")
    internal func tapsApart() {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.gestures
        gestures.tap(at: rig.point(2, 0), time: 10)
        gestures.tap(at: rig.point(2, 0), time: 11)
        gestures.tap(at: rig.point(20, 3), time: 11.1)
        #expect(rig.inputs.isEmpty)
    }

    @Test("a long press selects the word, a drag on extends it, and the menu shows when the finger lifts")
    internal func longPress() {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.gestures
        gestures.longPress(.began, at: rig.point(7, 0))
        #expect(rig.inputs == [.selectWord(TerminalRig.cell(7, 0))])
        rig.show([SelectedSpan(row: 0, first: 6, last: 10)])
        #expect(!rig.view.isMenuShown)
        // past the word's end: from its start; before its start: from its end; within it: the word
        gestures.longPress(.changed, at: rig.point(3, 1))
        gestures.longPress(.changed, at: rig.point(2, 0))
        gestures.longPress(.changed, at: rig.point(8, 0))
        #expect(
            rig.inputs(after: 1) == [
                .select(start: TerminalRig.cell(6, 0), end: TerminalRig.cell(3, 1), rectangle: false),
                .select(start: TerminalRig.cell(10, 0), end: TerminalRig.cell(2, 0), rectangle: false),
                .select(start: TerminalRig.cell(6, 0), end: TerminalRig.cell(10, 0), rectangle: false),
            ]
        )
        gestures.longPress(.ended, at: rig.point(8, 0))
        #expect(rig.view.isMenuShown)
    }

    @Test("dragging a handle moves its end of the selection")
    internal func handleDrag() throws {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.gestures
        gestures.longPress(.began, at: rig.point(2, 0))
        rig.show([SelectedSpan(row: 0, first: 0, last: 4)])
        gestures.longPress(.ended, at: rig.point(2, 0))
        let spots = try #require(rig.view.handleSpots)
        let before = rig.inputs.count
        // the finger lands on the end's knob and drags it two rows down
        let knob = spots.end.knob
        let row = rig.view.font.cellHeight
        gestures.pan(.began, at: CGPoint(x: knob.x, y: knob.y + 4), translation: CGPoint(x: 0, y: 4), velocity: .zero)
        #expect(!rig.view.isMenuShown)
        gestures.pan(
            .changed,
            at: CGPoint(x: knob.x, y: knob.y + 2 * row),
            translation: CGPoint(x: 0, y: 2 * row),
            velocity: .zero
        )
        gestures.pan(
            .ended,
            at: CGPoint(x: knob.x, y: knob.y + 2 * row),
            translation: CGPoint(x: 0, y: 2 * row),
            velocity: CGPoint(x: 0, y: 3_000)
        )
        #expect(
            rig.inputs(after: before)
                == [.select(start: TerminalRig.cell(0, 0), end: TerminalRig.cell(4, 2), rectangle: false)]
        )
        #expect(Self.turns(rig.inputs).isEmpty)
        #expect(!rig.view.pointer.isFlinging)
        #expect(rig.view.isMenuShown)
    }

    @Test("a pinch changes the text size in whole points, within the settings' range, and the grid with it")
    internal func pinch() {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.gestures
        let grid = rig.view.grid
        gestures.pinch(.began, scale: 1)
        gestures.pinch(.changed, scale: 1.03)
        #expect(rig.view.textSize == TerminalRig.textSize)
        gestures.pinch(.changed, scale: 1.5)
        #expect(rig.view.textSize == 20)
        rig.view.layoutIfNeeded()
        #expect(rig.view.grid != grid)
        #expect(rig.pane.sizes.last == rig.view.grid)
        gestures.pinch(.changed, scale: 10)
        #expect(rig.view.textSize == TerminalPointer.textSizes.upperBound)
        gestures.pinch(.ended, scale: 10)
        gestures.pinch(.began, scale: 1)
        gestures.pinch(.changed, scale: 0.01)
        #expect(rig.view.textSize == TerminalPointer.textSizes.lowerBound)
    }

    @Test("while a pinch runs, a drag scrolls nothing")
    internal func pinchHoldsTheDrag() {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.gestures
        let row = rig.view.font.cellHeight
        let point = rig.point(5, 5)
        gestures.pan(.began, at: point, translation: .zero, velocity: .zero)
        gestures.pinch(.began, scale: 1)
        gestures.pan(.changed, at: point, translation: CGPoint(x: 0, y: 3 * row), velocity: .zero)
        gestures.pan(.ended, at: point, translation: CGPoint(x: 0, y: 3 * row), velocity: CGPoint(x: 0, y: 3_000))
        #expect(Self.turns(rig.inputs).isEmpty)
        #expect(!rig.view.pointer.isFlinging)
    }
}
