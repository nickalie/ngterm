import Foundation
import NgtermTerminal
import Testing

/// What the session view takes from a terminal: a sideways swipe to the next or previous pane, a touch selecting the
/// pane, and its lines for VoiceOver (mobile/android's `TerminalGesturesTest` swipe cases and spoken text).
@MainActor
@Suite("the session view's hooks")
internal struct SessionHooksTests {
    // a rig whose swipes are recorded: `true` forward
    private static func swiping() -> (TerminalRig, () -> [Bool]) {
        let rig = TerminalRig(clipboard: nil)
        var swipes: [Bool] = []
        rig.view.onSwipe = { swipes.append($0) }
        return (rig, { swipes })
    }

    @Test("a sideways drag far enough moves to the next pane leftward and the previous rightward, scrolling nothing")
    internal func swipeFarEnough() {
        let (rig, swipes) = Self.swiping()
        let gestures = rig.view.gestures
        let start = rig.point(20, 5)
        gestures.pan(.began, at: start, translation: CGPoint(x: -12, y: 2), velocity: .zero)
        gestures.pan(.changed, at: start, translation: CGPoint(x: -40, y: 30), velocity: .zero)
        gestures.pan(.ended, at: start, translation: CGPoint(x: -90, y: 30), velocity: .zero)
        gestures.pan(.began, at: start, translation: CGPoint(x: 12, y: -1), velocity: .zero)
        gestures.pan(.ended, at: start, translation: CGPoint(x: 70, y: 0), velocity: .zero)
        #expect(swipes() == [true, false])
        #expect(rig.inputs.isEmpty, "a swipe scrolls nothing")
    }

    @Test("a short swipe moves on only when flung, the way it was flung; a cancelled one goes nowhere")
    internal func swipeFlung() {
        let (rig, swipes) = Self.swiping()
        let gestures = rig.view.gestures
        let start = rig.point(20, 5)
        gestures.pan(.began, at: start, translation: CGPoint(x: -12, y: 0), velocity: .zero)
        gestures.pan(.ended, at: start, translation: CGPoint(x: -20, y: 0), velocity: CGPoint(x: -100, y: 0))
        gestures.pan(.began, at: start, translation: CGPoint(x: -12, y: 0), velocity: .zero)
        gestures.pan(.ended, at: start, translation: CGPoint(x: -20, y: 0), velocity: CGPoint(x: 1_500, y: 0))
        gestures.pan(.began, at: start, translation: CGPoint(x: -12, y: 0), velocity: .zero)
        gestures.pan(.cancelled, at: start, translation: CGPoint(x: -200, y: 0), velocity: CGPoint(x: -2_000, y: 0))
        #expect(swipes() == [false])
    }

    @Test("a drag mostly down scrolls, and with no swipe to go to a sideways drag scrolls too")
    internal func dragsScroll() {
        let (rig, swipes) = Self.swiping()
        let gestures = rig.view.gestures
        let row = rig.view.font.cellHeight
        let start = rig.point(20, 5)
        gestures.pan(.began, at: start, translation: CGPoint(x: 6, y: 2 * row), velocity: .zero)
        gestures.pan(.ended, at: start, translation: CGPoint(x: 90, y: 2 * row), velocity: .zero)
        #expect(swipes().isEmpty)
        #expect(!rig.inputs.isEmpty, "the drag scrolled")

        let plain = TerminalRig(clipboard: nil)
        plain.view.gestures.pan(.began, at: start, translation: CGPoint(x: -20, y: row), velocity: .zero)
        plain.view.gestures.pan(.ended, at: start, translation: CGPoint(x: -90, y: row), velocity: .zero)
        #expect(!plain.inputs.isEmpty, "with no swipe, the drag scrolls")
    }

    @Test("a touch on the terminal selects it")
    internal func touchSelects() {
        let rig = TerminalRig(clipboard: nil)
        var selected = 0
        rig.view.onSelect = { selected += 1 }
        rig.view.touchesBegan([], with: nil)
        #expect(selected == 1)
    }

    @Test("VoiceOver reads the screen line by line, each line framed on its row, the page down to its last text")
    internal func lines() {
        let rig = TerminalRig(clipboard: nil)
        let view = rig.view
        #expect(view.accessibilityPageContent() == "hello world\nsecond line\n\nthe end")
        #expect(view.accessibilityValue == view.accessibilityPageContent())
        #expect(view.accessibilityContent(forLineNumber: 1) == "second line")
        #expect(view.accessibilityContent(forLineNumber: 2)?.isEmpty == true)
        #expect(view.accessibilityContent(forLineNumber: 99) == nil)
        #expect(view.accessibilityFrame(forLineNumber: 3).height == view.font.cellHeight)
        #expect(view.accessibilityLineNumber(for: rig.point(3, 1)) == 1)
        #expect(view.accessibilityLineNumber(for: CGPoint(x: -5, y: -5)) == NSNotFound)
    }
}
