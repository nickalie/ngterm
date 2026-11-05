import CoreGraphics
import NgtermCore
import NgtermCoreFakes
import NgtermTerminal
import Testing
import UIKit

/// A mouse's and a trackpad's input against the fake pane, which records it (mobile/android's `TerminalGesturesTest`,
/// its mouse half).
@MainActor
@Suite("a pointer's gestures")
internal struct TerminalPointerGesturesTests {
    private static func mouse(
        _ rig: TerminalRig,
        _ action: PaneMouseAction,
        _ button: PaneMouseButton?,
        at point: CGPoint,
        mods: UInt8 = 0
    ) -> FakeInput {
        let pixels = rig.pixels(point)
        return .mouse(PaneMouse(action: action, button: button, mods: mods, x: pixels.x, y: pixels.y))
    }

    @Test("the mouse goes to a program tracking it: press, a motion per new cell, release")
    internal func tracking() {
        let rig = TerminalRig(clipboard: nil)
        rig.pane.tracking = true
        let gestures = rig.view.pointerGestures
        let start = rig.point(2, 1)
        let sameCell = CGPoint(x: start.x + 1, y: start.y)
        let next = rig.point(5, 1)
        gestures.press(.began, at: start, buttons: .primary, mods: .control, time: 1)
        gestures.press(.changed, at: sameCell, buttons: .primary, mods: .control, time: 1)
        gestures.press(.changed, at: next, buttons: .primary, mods: .control, time: 1)
        gestures.press(.ended, at: next, buttons: .primary, mods: [], time: 1)
        #expect(
            rig.inputs == [
                Self.mouse(rig, .press, .left, at: start, mods: 2),
                Self.mouse(rig, .motion, .left, at: next, mods: 2),
                Self.mouse(rig, .release, .left, at: next),
            ]
        )
        // the secondary button too, and a press the system cancels still releases
        gestures.press(.began, at: start, buttons: .secondary, mods: [], time: 2)
        gestures.press(.cancelled, at: start, buttons: .secondary, mods: [], time: 2)
        #expect(
            rig.inputs(after: 3) == [
                Self.mouse(rig, .press, .right, at: start),
                Self.mouse(rig, .release, .right, at: start),
            ]
        )
        #expect(!rig.view.isMenuShown)
    }

    @Test("the mouse selects by drag, double click and triple click; shift overrides tracking")
    internal func selects() {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.pointerGestures
        let start = rig.point(1, 0)
        gestures.press(.began, at: start, buttons: .primary, mods: [], time: 1)
        gestures.press(.changed, at: rig.point(6, 1), buttons: .primary, mods: [], time: 1)
        gestures.press(.ended, at: rig.point(6, 1), buttons: .primary, mods: [], time: 1)
        #expect(
            rig.inputs == [
                .clearSelection,
                .select(start: TerminalRig.cell(1, 0), end: TerminalRig.cell(6, 1), rectangle: false),
            ]
        )
        // a double and a triple click, with shift held over a program that tracks the mouse
        rig.pane.tracking = true
        for time in [5, 5.2, 5.4] {
            gestures.press(.began, at: start, buttons: .primary, mods: .shift, time: time)
            gestures.press(.ended, at: start, buttons: .primary, mods: .shift, time: time)
        }
        #expect(
            rig.inputs(after: 2) == [
                .clearSelection,
                .selectWord(TerminalRig.cell(1, 0)),
                .selectLine(TerminalRig.cell(1, 0)),
            ]
        )
        #expect(!rig.view.selection.isTouchSelection)
    }

    @Test("the secondary button opens the edit menu where it clicked")
    internal func secondaryClick() {
        let rig = TerminalRig(clipboard: nil)
        rig.view.pointerGestures.press(.began, at: rig.point(4, 2), buttons: .secondary, mods: [], time: 1)
        #expect(rig.view.isMenuShown)
        #expect(rig.inputs.isEmpty)
    }

    @Test("the wheel and a two-finger scroll turn the pane's wheel by whole rows and columns, with the modifiers")
    internal func scroll() {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.pointerGestures
        let font = rig.view.font
        let point = rig.point(5, 5)
        gestures.scroll(.began, translation: .zero, at: point, mods: [])
        gestures.scroll(.changed, translation: CGPoint(x: 0, y: 2.5 * font.cellHeight), at: point, mods: .alternate)
        gestures.scroll(
            .changed,
            translation: CGPoint(x: -3 * font.cellWidth, y: 2.5 * font.cellHeight),
            at: point,
            mods: []
        )
        gestures.scroll(.cancelled, translation: CGPoint(x: 0, y: 99 * font.cellHeight), at: point, mods: [])
        let turns: [PaneWheel] = rig.inputs.compactMap { input in
            if case .wheel(let wheel) = input {
                return wheel
            }
            return nil
        }
        #expect(turns.map(\.rows) == [2, 0])
        #expect(turns.map(\.columns) == [0, 3])
        #expect(turns.map(\.mods) == [4, 0])
    }

    @Test("the pointer's motion goes to a program tracking the mouse, once per new cell")
    internal func hover() {
        let rig = TerminalRig(clipboard: nil)
        let gestures = rig.view.pointerGestures
        let point = rig.point(3, 3)
        gestures.hover(at: point, mods: [])
        #expect(rig.inputs.isEmpty)
        rig.pane.tracking = true
        gestures.hover(at: point, mods: .command)
        gestures.hover(at: CGPoint(x: point.x + 1, y: point.y), mods: [])
        gestures.hover(at: rig.point(4, 3), mods: [])
        #expect(
            rig.inputs == [
                Self.mouse(rig, .motion, nil, at: point, mods: 8),
                Self.mouse(rig, .motion, nil, at: rig.point(4, 3)),
            ]
        )
    }
}
