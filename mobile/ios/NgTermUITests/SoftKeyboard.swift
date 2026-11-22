import XCTest

/// The wait of the soft keyboard tests for a soft keyboard whose keys can be tapped.
internal enum SoftKeyboard {
    // a letter of the soft keyboard: on the screen once the keyboard is, as every letter the tests tap
    private static let readinessKey = "c"
    // how far a frame's edge may lie past the screen's, in points, from rounding
    private static let onScreenSlack: CGFloat = 1
    private static let springboardIdentifier = "com.apple.springboard"

    /// Waits up to `wait` seconds until the soft keyboard's keys are on the screen, and returns the keyboard.
    ///
    /// It returns nil, with the test failed, when the keys stay below the screen or the keyboard has no such key. A
    /// keyboard in the hierarchy is not yet one on the screen: the system keeps it below the screen while it takes a
    /// hardware keyboard as attached, and a key there can't be tapped. It skips only when reads completed and none
    /// found a keyboard: a wait on time alone can end with no read on a loaded runner, and the test fails then.
    @MainActor
    internal static func shown(in app: XCUIApplication, lasting wait: TimeInterval) throws -> XCUIElement? {
        let keyboard = app.keyboards.firstMatch
        let key = keyboard.keys[readinessKey]
        // SpringBoard's frame is the screen's: the app's is only its window's, smaller than the screen on an iPad
        let springboard = XCUIApplication(bundleIdentifier: springboardIdentifier)
        var screenSeen: CGRect?
        var keyboardSeen = false
        var keySeen: CGRect?
        let shown = ScreenRead.Wait(reads: 1, lasting: wait).poll {
            let keyboardShown = ScreenRead.isShown(keyboard)
            guard keyboardShown == .holds else {
                return keyboardShown
            }
            keyboardSeen = true
            if screenSeen == nil {
                _ = ScreenRead.shows(springboard) { snapshot in
                    screenSeen = snapshot.frame.insetBy(dx: -onScreenSlack, dy: -onScreenSlack)
                    return true
                }
            }
            guard let screen = screenSeen else {
                return .failed
            }
            return ScreenRead.shows(key) { snapshot in
                keySeen = snapshot.frame
                return screen.contains(snapshot.frame)
            }
        }
        if shown.held {
            return keyboard
        }
        guard shown.reads > 0 else {
            FakeApp.attachScreen("no read of the soft keyboard")
            XCTFail("no read of the soft keyboard completed in \(Int(shown.seconds)) s (\(shown.failed) failed)")
            return nil
        }
        guard keyboardSeen else {
            throw XCTSkip("the simulator has the Mac's keyboard attached, so no soft keyboard shows")
        }
        guard let keySeen, let screen = screenSeen else {
            FakeApp.attachScreen("the soft keyboard with no '\(readinessKey)' key")
            XCTFail("the soft keyboard shows, but no read found its '\(readinessKey)' key on the screen")
            return nil
        }
        FakeApp.attachScreen("the soft keyboard below the screen")
        XCTFail(
            "the soft keyboard's '\(readinessKey)' stays at \(keySeen), off the screen \(screen): the simulator "
                + "takes a hardware keyboard as attached (AutomaticMinimizationEnabled, which build-ios.sh clears "
                + "before a boot)"
        )
        return nil
    }
}
