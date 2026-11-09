import XCTest

/// Typing into a terminal on the fake core, through the system's own input (mobile/android's `TerminalKeyboardTest`).
///
/// The soft keyboard and the extra keys, and a hardware keyboard's keys and chords. The app shows a terminal on a fake
/// pane alone (`-ngterm-fake-terminal`), with what the pane got written over it.
internal final class KeyboardTests: XCTestCase {
    private static let wait: TimeInterval = 15
    // how long the slide-to-type introduction gets to come up after the keyboard, which it follows within a moment
    private static let introductionWait: TimeInterval = 5

    // the extra key named `name`
    @MainActor
    private static func key(_ name: String, in app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any)["extra-key-\(name)"]
    }

    // Closes the system's slide-to-type introduction if it came up over the soft keyboard. Its Continue is the only way
    // past it: it covers the keyboard's lower half and the extra keys, so every tap lands on it and types nothing.
    // build-ios.sh sets the preference that should stop it, but a simulator's first keyboard now and then shows it
    // anyway.
    @MainActor
    private static func closeTheKeyboardIntroduction(in app: XCUIApplication) {
        let introduction = app.descendants(matching: .any)["UIContinuousPathIntroductionView"]
        guard introduction.waitForExistence(timeout: introductionWait) else {
            return
        }
        introduction.buttons["Continue"].tap()
        XCTAssertTrue(introduction.waitForNonExistence(timeout: wait), "the slide-to-type introduction stayed")
    }

    // waits until the bench's `line` reads `text`
    @MainActor
    private static func expect(_ line: String, reads text: String, in app: XCUIApplication) {
        let reading = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label == %@", text),
            object: app.staticTexts[line]
        )
        let result = XCTWaiter.wait(for: [reading], timeout: wait)
        XCTAssertEqual(result, .completed, "\(line) reads '\(app.staticTexts[line].label)', not '\(text)'")
    }

    @MainActor
    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-ngterm-fake-core", "-ngterm-fake-terminal"]
        app.launch()
        // a tap on the terminal gives it the focus and opens the soft keyboard
        app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        FakeApp.expect(Self.key("escape", in: app), "the extra keys")
        return app
    }

    @MainActor
    internal func testTheSoftKeyboardAndTheExtraKeysType() throws {
        let app = launch()
        let keyboard = app.keyboards.firstMatch
        guard keyboard.waitForExistence(timeout: Self.wait) else {
            throw XCTSkip("the simulator has the Mac's keyboard attached, so no soft keyboard shows")
        }
        Self.closeTheKeyboardIntroduction(in: app)
        keyboard.keys["l"].tap()
        keyboard.keys["s"].tap()
        Self.expect("bench-typed", reads: "ls", in: app)
        Self.key("control", in: app).tap()
        XCTAssertEqual(Self.key("control", in: app).value as? String, "On for the next key")
        keyboard.keys["c"].tap()
        Self.key("escape", in: app).tap()
        Self.key("upArrow", in: app).tap()
        Self.key("pipe", in: app).tap()
        Self.expect("bench-keys", reads: "key_c/2 escape/0 arrow_up/0", in: app)
        Self.expect("bench-typed", reads: "ls|", in: app)
        XCTAssertEqual(Self.key("control", in: app).value as? String, "Off")
        // the keyboard's key closes the soft keyboard, and the extra keys stay
        Self.key("keyboard", in: app).tap()
        XCTAssertTrue(keyboard.waitForNonExistence(timeout: Self.wait))
        XCTAssertTrue(Self.key("escape", in: app).exists)
        Self.key("keyboard", in: app).tap()
        XCTAssertTrue(keyboard.waitForExistence(timeout: Self.wait))
    }

    @MainActor
    internal func testAHardwareKeyboardTypesAndItsChordsRunInTheApp() {
        let app = launch()
        app.typeKey("e", modifierFlags: [])
        app.typeKey("c", modifierFlags: .control)
        app.typeKey(XCUIKeyboardKey.leftArrow.rawValue, modifierFlags: [])
        Self.expect("bench-typed", reads: "e", in: app)
        Self.expect("bench-keys", reads: "key_c/2 arrow_left/0", in: app)
        let grid = app.staticTexts["bench-grid"].label
        app.typeKey("=", modifierFlags: .command)
        let bigger = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label != %@", grid),
            object: app.staticTexts["bench-grid"]
        )
        XCTAssertEqual(XCTWaiter.wait(for: [bigger], timeout: Self.wait), .completed, "⌘= made the text no bigger")
        app.typeKey("0", modifierFlags: .command)
        Self.expect("bench-grid", reads: grid, in: app)
        app.typeKey(XCUIKeyboardKey.rightArrow.rawValue, modifierFlags: [.command, .option])
        app.typeKey(XCUIKeyboardKey.upArrow.rawValue, modifierFlags: [.command, .option])
        Self.expect("bench-moves", reads: "next pane, previous session", in: app)
        Self.expect("bench-keys", reads: "key_c/2 arrow_left/0", in: app)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
