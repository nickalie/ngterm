import XCTest

/// Scrolls of a form (Settings' list) that bring a row wholly into view before a press or tap.
internal enum FormScroll {
    // a drag's reach, as a share of the form's height; its distance from the leading edge; its press and hold
    private static let dragReach: CGFloat = 0.35
    private static let dragEdge: CGFloat = 0.08
    private static let dragPress: TimeInterval = 0.05
    private static let dragHold: TimeInterval = 0.3
    private static let half: CGFloat = 0.5

    /// Scrolls `form` until all of `element` is in it: swipes up until the list holds it, then drags it to the middle.
    ///
    /// Hittable is not enough: a row with only a sliver above the foot of an iPad's sheet is hittable, and a press at
    /// its hit point lands outside the sheet and closes it. A swipe's momentum carries a row past the whole view, so
    /// the last moves are drags that end held. A row that the list held and then let go went above the view.
    @MainActor
    internal static func scroll(to element: XCUIElement, in form: XCUIElement, swipes: Int) {
        var held = false
        for _ in 0..<swipes {
            if isWhollyShown(element, in: form) {
                break
            }
            guard element.exists else {
                if held {
                    drag(form, by: form.frame.height)
                } else {
                    form.swipeUp()
                }
                continue
            }
            held = true
            drag(form, by: form.frame.midY - element.frame.midY)
        }
        XCTAssertTrue(isWhollyShown(element, in: form), "\(element) never came wholly into view")
    }

    // drags at the form's leading edge, clear of its switches, and holds at the end so the list stops there
    @MainActor
    private static func drag(_ form: XCUIElement, by distance: CGFloat) {
        let reach = max(-dragReach, min(dragReach, distance / form.frame.height))
        let start = form.coordinate(withNormalizedOffset: CGVector(dx: dragEdge, dy: half - reach * half))
        let end = form.coordinate(withNormalizedOffset: CGVector(dx: dragEdge, dy: half + reach * half))
        start.press(forDuration: dragPress, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: dragHold)
    }

    @MainActor
    private static func isWhollyShown(_ element: XCUIElement, in form: XCUIElement) -> Bool {
        element.exists && element.isHittable && form.frame.contains(element.frame)
    }
}
