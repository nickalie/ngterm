import XCTest

/// The wait of the soft keyboard tests for a soft keyboard whose keys can be tapped.
internal enum SoftKeyboard {
    /// What the reads of one wait found.
    private struct Seen {
        var keyboard = false
        var screen: CGRect?
        var screenFailed = 0
        var key: CGRect?
        var keyReads = 0
        var keyFailed = 0
    }

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
    /// found a keyboard: a wait on time alone can end with no read on a loaded runner, and the test fails then. The
    /// failure tells which read (the keyboard, SpringBoard's frame, the key) did not complete.
    @MainActor
    internal static func shown(in app: XCUIApplication, lasting wait: TimeInterval) throws -> XCUIElement? {
        let keyboard = app.keyboards.firstMatch
        let key = keyboard.keys[readinessKey]
        // SpringBoard's frame is the screen's: the app's is only its window's, smaller than the screen on an iPad
        let springboard = XCUIApplication(bundleIdentifier: springboardIdentifier)
        var seen = Seen()
        let shown = ScreenRead.Wait(reads: 1, lasting: wait).poll {
            let keyboardShown = ScreenRead.isShown(keyboard)
            guard keyboardShown == .holds else {
                return keyboardShown
            }
            seen.keyboard = true
            if seen.screen == nil {
                _ = ScreenRead.shows(springboard) { snapshot in
                    seen.screen = snapshot.frame.insetBy(dx: -onScreenSlack, dy: -onScreenSlack)
                    return true
                }
            }
            guard let screen = seen.screen else {
                seen.screenFailed += 1
                return .failed
            }
            let keyRead = ScreenRead.shows(key) { snapshot in
                seen.key = snapshot.frame
                return screen.contains(snapshot.frame)
            }
            if keyRead == .failed {
                seen.keyFailed += 1
            } else {
                seen.keyReads += 1
            }
            return keyRead
        }
        if shown.held {
            return keyboard
        }
        guard let failure = failure(of: seen, after: shown) else {
            throw XCTSkip("the simulator has the Mac's keyboard attached, so no soft keyboard shows")
        }
        FakeApp.attachScreen(failure.screen)
        XCTFail(failure.message)
        return nil
    }

    /// The screen's name and the failure for a wait that did not end with the key on the screen: nil when reads
    /// completed and none found a keyboard.
    private static func failure(of seen: Seen, after shown: ScreenRead.Polled) -> (screen: String, message: String)? {
        let time = "in \(Int(shown.seconds)) s"
        let key = "'\(readinessKey)'"
        guard seen.keyboard else {
            guard shown.reads > 0 else {
                return (
                    screen: "no read of the soft keyboard",
                    message: "no read of the soft keyboard completed \(time) (\(shown.failed) failed)"
                )
            }
            return nil
        }
        guard let screen = seen.screen else {
            return (
                screen: "no read of the screen's frame",
                message: "the soft keyboard shows, but no read of SpringBoard's frame completed \(time) "
                    + "(\(seen.screenFailed) failed)"
            )
        }
        guard seen.keyReads > 0 else {
            return (
                screen: "no read of the \(key) key",
                message: "the soft keyboard shows, but no read of its \(key) key completed \(time) "
                    + "(\(seen.keyFailed) failed)"
            )
        }
        guard let frame = seen.key else {
            return (
                screen: "the soft keyboard with no \(key) key",
                message: "the soft keyboard shows, but \(seen.keyReads) reads found no \(key) key in it"
            )
        }
        return (
            screen: "the soft keyboard below the screen",
            message: "the soft keyboard's \(key) stays at \(frame), off the screen \(screen): the simulator takes a "
                + "hardware keyboard as attached (AutomaticMinimizationEnabled, which build-ios.sh clears before a "
                + "boot)"
        )
    }
}
