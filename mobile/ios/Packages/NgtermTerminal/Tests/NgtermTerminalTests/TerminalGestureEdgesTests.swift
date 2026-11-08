import NgtermCore
import NgtermCoreFakes
import NgtermTerminal
import Testing
import UIKit

/// The gestures at their edges against the fake pane: a fling running on in a window, what the system asks the
/// gestures' delegate, the middle button, the pointer's look, and input before the view has a grid.
@MainActor
@Suite("gestures at their edges")
internal struct TerminalGestureEdgesTests {
    // how often a wait looks at its condition, and how many times (about 10 s on an idle simulator)
    private static let stepMilliseconds = 20
    private static let looks = 500
    private static let middleButton = 3

    // waits for `condition`, failing the test when it doesn't come
    private static func until(_ condition: () -> Bool) async throws {
        var looked = 0
        while !condition(), looked < looks {
            looked += 1
            try await Task.sleep(for: .milliseconds(stepMilliseconds))
        }
        try #require(condition(), "timed out waiting")
    }

    // the rows of the wheel turns a fake pane took
    private static func turns(_ inputs: [FakeInput]) -> [Int32] {
        inputs.compactMap { input in
            if case .wheel(let wheel) = input {
                return wheel.rows
            }
            return nil
        }
    }

    // a pointer event at `point` as the pane takes it
    private static func mouse(
        _ rig: TerminalRig,
        _ action: PaneMouseAction,
        _ button: PaneMouseButton?,
        at point: CGPoint
    ) -> FakeInput {
        let pixels = rig.pixels(point)
        return .mouse(PaneMouse(action: action, button: button, mods: 0, x: pixels.x, y: pixels.y))
    }

    // a view with no size yet, so no grid
    private static func gridless(_ pane: FakePane) -> TerminalView {
        TerminalView(pane: pane, palette: .copper, textSize: TerminalRig.textSize, clipboard: MemoryClipboard(nil))
    }

    @Test("a flick in a window scrolls on by itself, the rows coming down, until it slows to a stop")
    internal func flingInWindow() async throws {
        let rig = TerminalRig(clipboard: nil)
        let window = try TestWindow.make(rig.view.frame)
        window.addSubview(rig.view)
        defer { rig.view.removeFromSuperview() }
        let point = rig.point(5, 5)
        let velocity: CGFloat = 300
        rig.view.gestures.pan(.began, at: point, translation: .zero, velocity: .zero)
        rig.view.gestures.pan(.ended, at: point, translation: .zero, velocity: CGPoint(x: 0, y: velocity))
        #expect(rig.view.pointer.isFlinging)
        try await Self.until { !rig.view.pointer.isFlinging }
        let turns = Self.turns(rig.inputs)
        #expect(turns.count > 1, "a turn per row as the fling goes")
        #expect(turns.allSatisfy { $0 > 0 })
        // no farther than the whole fling goes
        let farthest = FlingDecay(velocity: velocity).offset(after: .infinity) / rig.view.font.cellHeight
        #expect(CGFloat(turns.reduce(0, +)) <= farthest.rounded(.up))
    }

    @Test("a fling stops as the view leaves its window")
    internal func flingLeavesWindow() throws {
        let rig = TerminalRig(clipboard: nil)
        let window = try TestWindow.make(rig.view.frame)
        window.addSubview(rig.view)
        rig.view.pointer.fling(velocity: 2_000, at: rig.point(5, 5))
        #expect(rig.view.pointer.isFlinging)
        rig.view.removeFromSuperview()
        #expect(!rig.view.pointer.isFlinging)
    }

    @Test("a finger's touch stops a fling before any gesture takes it, and every gesture gets the touch")
    internal func touchStopsFling() {
        let rig = TerminalRig(clipboard: nil)
        rig.view.pointer.fling(velocity: 2_000, at: rig.point(5, 5))
        #expect(rig.view.pointer.isFlinging)
        #expect(rig.view.gestures.gestureRecognizer(UITapGestureRecognizer(), shouldReceive: UITouch()))
        #expect(!rig.view.pointer.isFlinging)
    }

    @Test("a long press doesn't begin on a selection handle, and begins anywhere while there are none")
    internal func longPressOnHandle() throws {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.gestures
        let press = try #require(rig.view.gestureRecognizers?.first { $0 is UILongPressGestureRecognizer })
        #expect(gestures.gestureRecognizerShouldBegin(press))
        // a selection from the top left cell, where the press is
        gestures.longPress(.began, at: rig.point(2, 0))
        rig.show([SelectedSpan(row: 0, first: 0, last: 4)])
        gestures.longPress(.ended, at: rig.point(2, 0))
        let spots = try #require(rig.view.handleSpots)
        #expect(spots.nearest(to: press.location(in: rig.view)) != nil)
        #expect(!gestures.gestureRecognizerShouldBegin(press), "the finger takes the handle")
    }

    @Test("a pinch and a drag go together; no other two gestures do")
    internal func simultaneous() {
        let gestures = TerminalRig(clipboard: nil).view.gestures
        let pinch = UIPinchGestureRecognizer()
        let pan = UIPanGestureRecognizer()
        let tap = UITapGestureRecognizer()
        let press = UILongPressGestureRecognizer()
        #expect(gestures.gestureRecognizer(pinch, shouldRecognizeSimultaneouslyWith: pan))
        #expect(gestures.gestureRecognizer(pan, shouldRecognizeSimultaneouslyWith: pinch))
        #expect(!gestures.gestureRecognizer(tap, shouldRecognizeSimultaneouslyWith: pan))
        #expect(!gestures.gestureRecognizer(pinch, shouldRecognizeSimultaneouslyWith: press))
        #expect(!gestures.gestureRecognizer(pan, shouldRecognizeSimultaneouslyWith: pan))
    }

    @Test("a long press before the view has a grid selects nothing, and a lift with no press before it does nothing")
    internal func longPressWithoutGrid() {
        let pane = FakePane { _ in RowBufferWriter(columns: 4, rows: 2).bytes() }
        let view = Self.gridless(pane)
        view.gestures.longPress(.began, at: CGPoint(x: 5, y: 5))
        #expect(pane.inputs.isEmpty)
        #expect(!view.selection.isTouchSelection)
        let rig = TerminalRig(clipboard: nil)
        rig.view.gestures.longPress(.ended, at: rig.point(2, 0))
        #expect(rig.inputs.isEmpty)
        #expect(!rig.view.isMenuShown)
    }

    @Test("the pointer is a text beam over the terminal, and the system's own while a program takes the mouse")
    internal func pointerStyle() {
        let rig = TerminalRig(clipboard: nil)
        let interaction = UIPointerInteraction(delegate: nil)
        let region = UIPointerRegion(rect: rig.view.bounds, identifier: nil)
        #expect(rig.view.pointerGestures.pointerInteraction(interaction, styleFor: region) != nil)
        rig.pane.tracking = true
        #expect(rig.view.pointerGestures.pointerInteraction(interaction, styleFor: region) == nil)
    }

    @Test("the middle button goes to a program tracking the mouse, and otherwise does nothing")
    internal func middleButton() {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.pointerGestures
        let point = rig.point(3, 1)
        let middle = UIEvent.ButtonMask.button(Self.middleButton)
        gestures.press(.began, at: point, buttons: middle, mods: [], time: 1)
        gestures.press(.ended, at: point, buttons: middle, mods: [], time: 1)
        #expect(rig.inputs.isEmpty)
        #expect(!rig.view.isMenuShown)
        rig.pane.tracking = true
        gestures.press(.began, at: point, buttons: middle, mods: [], time: 2)
        gestures.press(.ended, at: point, buttons: middle, mods: [], time: 2)
        #expect(
            rig.inputs == [Self.mouse(rig, .press, .middle, at: point), Self.mouse(rig, .release, .middle, at: point)]
        )
    }

    @Test("a press before the view has a grid, and a drag with no press before it, do nothing")
    internal func pressWithoutGrid() {
        let pane = FakePane { _ in RowBufferWriter(columns: 4, rows: 2).bytes() }
        let view = Self.gridless(pane)
        view.pointerGestures.press(.began, at: CGPoint(x: 5, y: 5), buttons: .primary, mods: [], time: 1)
        #expect(pane.inputs.isEmpty)
        let rig = TerminalRig(clipboard: nil)
        rig.view.pointerGestures.press(.changed, at: rig.point(6, 2), buttons: .primary, mods: [], time: 1)
        rig.view.pointerGestures.press(.ended, at: rig.point(6, 2), buttons: .primary, mods: [], time: 1)
        #expect(rig.inputs.isEmpty)
    }
}
