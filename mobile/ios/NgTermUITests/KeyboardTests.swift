import XCTest

/// Typing into a terminal on the fake core, through the system's own input (mobile/android's `TerminalKeyboardTest`).
///
/// The soft keyboard and the extra keys, and a hardware keyboard's keys and chords. The app shows a terminal on a fake
/// pane alone (`-ngterm-fake-terminal`), with what the pane got written over it. Its keyboard has no hardware keyboard,
/// so the extra keys show on a simulator that has the Mac's: a session on a real machine hides them there.
internal final class KeyboardTests: XCTestCase {
    private static let wait: TimeInterval = 15
    // how long the slide-to-type introduction gets to come up after the keyboard, which it follows within a moment
    private static let introductionWait: TimeInterval = 5
    // a letter of the soft keyboard: on the screen once the keyboard is, as every letter the tests tap
    private static let readinessKey = "c"
    // how far a frame's edge may lie past the screen's, in points, from rounding
    private static let onScreenSlack: CGFloat = 1
    private static let springboardIdentifier = "com.apple.springboard"
    // how far apart a wait reads a line of the bench, which draws its lines again five times a second
    private static let readBeat: TimeInterval = 0.2
    // a touch held a moment: longer than an arrow's hold (100 ms), shorter than its first repeat (400 ms)
    private static let holdPress: TimeInterval = 0.25
    // an arrow held long enough to repeat, and the least keys it sends so: its hold, its first repeat at 400 ms, and
    // one every 50 ms after
    private static let longPress: TimeInterval = 0.8
    private static let leastRepeats = 4
    private static let settle: TimeInterval = 1
    // how long the system gets to take a swipe from the screen's foot and show the Home Screen
    private static let awayWait: TimeInterval = 3
    // a short swipe inside a key, from one side to the other: more than 10 points on every key
    private static let swipeFrom: CGFloat = 0.15
    private static let swipeTo: CGFloat = 0.85
    // how long a swipe's touch stays before it moves
    private static let quickPress: TimeInterval = 0.01

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
        FakeApp.expectGone(introduction, "the slide-to-type introduction", lasting: wait)
    }

    // Waits until the soft keyboard's keys are on the screen, and returns the keyboard: nil, with the test failed, when
    // they stay below it. A keyboard in the hierarchy is not yet one on the screen: the system keeps it below the
    // screen while it takes a hardware keyboard as attached, and a key there can't be tapped. Only reads that completed
    // and found no keyboard at all are a reason to skip: a wait on time alone can end with no read on a loaded runner.
    @MainActor
    private static func softKeyboard(in app: XCUIApplication) throws -> XCUIElement? {
        let keyboard = app.keyboards.firstMatch
        // SpringBoard's frame is the screen's: the app's is only its window's, smaller than the screen on an iPad
        let screen = XCUIApplication(bundleIdentifier: springboardIdentifier).frame
            .insetBy(dx: -onScreenSlack, dy: -onScreenSlack)
        var seen: CGRect?
        let shown = ScreenRead.Wait(reads: 1, lasting: wait).poll {
            ScreenRead.shows(keyboard.keys[readinessKey]) { key in
                seen = key.frame
                return screen.contains(key.frame)
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
        guard let seen else {
            throw XCTSkip("the simulator has the Mac's keyboard attached, so no soft keyboard shows")
        }
        FakeApp.attachScreen("the soft keyboard below the screen")
        XCTFail(
            "the soft keyboard's '\(readinessKey)' stays at \(seen), off the screen \(screen): the simulator takes a "
                + "hardware keyboard as attached (AutomaticMinimizationEnabled, which build-ios.sh clears before a "
                + "boot)"
        )
        return nil
    }

    // Reads the bench's `line` again until its text `matches` or the wait ends, and returns the last text read: nil
    // when no read found the line (no read completed, or none found it). The test reads the line itself: an
    // XCTNSPredicateExpectation that reads the line once before the app updates it then captures the element's debug
    // description, and on a loaded simulator that capture can use all the wait, so the expectation reads the line no
    // second time. The wait is a number of reads, as FakeApp's waits are.
    @MainActor
    private static func read(_ line: String, in app: XCUIApplication, until matches: (String) -> Bool) -> String? {
        let element = app.staticTexts[line]
        var text: String?
        ScreenRead.Wait(lasting: wait, beat: readBeat).poll {
            ScreenRead.shows(element) { snapshot in
                text = snapshot.label
                return matches(snapshot.label)
            }
        }
        return text
    }

    // waits until the bench's `line` reads `text`
    @MainActor
    private static func expect(_ line: String, reads text: String, in app: XCUIApplication) {
        let label = read(line, in: app) { $0 == text }
        XCTAssertEqual(label, text, "\(line) reads '\(label ?? "nothing: no read found it")', not '\(text)'")
    }

    // the middle of the extra key `name` (or a point `across` it), or of the terminal, high over the keys, for nil
    @MainActor
    private static func point(_ name: String?, in app: XCUIApplication, across: CGFloat = 0.5) -> XCUICoordinate {
        guard let name else {
            return app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
        }
        return key(name, in: app).coordinate(withNormalizedOffset: CGVector(dx: across, dy: 0.5))
    }

    // a touch on the extra key `name`, held a moment, then slid onto the key `other` (the terminal for nil) and lifted
    // there
    @MainActor
    private static func slide(_ name: String, to other: String?, in app: XCUIApplication) {
        point(name, in: app).press(forDuration: holdPress, thenDragTo: point(other, in: app))
    }

    // a quick, short swipe from one side of the extra key `name` to the other, lifted on the key: past the slop before
    // a repeating key's hold acts
    @MainActor
    private static func swipeInside(_ name: String, in app: XCUIApplication) {
        let end = point(name, in: app, across: swipeTo)
        let start = point(name, in: app, across: swipeFrom)
        start.press(forDuration: quickPress, thenDragTo: end, withVelocity: .fast, thenHoldForDuration: 0)
    }

    // the screen, kept with the test's result
    @MainActor
    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    @MainActor
    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-ngterm-fake-core", "-ngterm-fake-terminal", FakeApp.stillArgument]
        app.launch()
        // a tap on the terminal gives it the focus and opens the soft keyboard
        app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        FakeApp.expect(Self.key("escape", in: app), "the extra keys")
        return app
    }

    @MainActor
    internal func testTheSoftKeyboardAndTheExtraKeysType() throws {
        let app = launch()
        guard let keyboard = try Self.softKeyboard(in: app) else {
            return
        }
        Self.closeTheKeyboardIntroduction(in: app)
        // each key reaches the program as it goes down, not when a space or Return ends the word
        keyboard.keys["l"].tap()
        Self.expect("bench-typed", reads: "l", in: app)
        keyboard.keys["s"].tap()
        Self.expect("bench-typed", reads: "ls", in: app)
        // a swipe that starts on a key, here from Esc up over the terminal, sends no key: bench-keys below has one
        // Escape, the tap's
        let terminal = app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
        let escape = Self.key("escape", in: app).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        escape.press(forDuration: 0.2, thenDragTo: terminal)
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
        FakeApp.expectGone(keyboard, "the soft keyboard", lasting: Self.wait)
        XCTAssertTrue(Self.key("escape", in: app).exists)
        // at the screen's foot, over the home indicator, a tap on a key is the key's, not the system's
        Self.key("downArrow", in: app).tap()
        Self.expect("bench-keys", reads: "key_c/2 escape/0 arrow_up/0 arrow_down/0", in: app)
        Self.key("keyboard", in: app).tap()
        FakeApp.expect(keyboard, "the soft keyboard again", lasting: Self.wait)
    }

    // A key acts as the finger lifts on it. A touch that slides off a key, a swipe that starts on a key or a short
    // swipe inside one sends nothing and toggles no Ctrl or Alt; an arrow held 100 ms acts once, then repeats from
    // 400 ms. The keys the pane got, in order, are in bench-keys: each step adds its own to `sent`.
    @MainActor
    internal func testAnExtraKeyActsOnTheLiftAndASlideOrASwipeSendsNothing() throws {
        let app = launch()
        guard let keyboard = try Self.softKeyboard(in: app) else {
            return
        }
        Self.closeTheKeyboardIntroduction(in: app)
        var sent = tapsAndSlides(in: app)
        sent = modifiers(after: sent, keyboard: keyboard, in: app)
        sent = heldArrows(after: sent, in: app)
        theKeyboardsKeyAndTheFoot(after: sent, keyboard: keyboard, in: app)
    }

    // a tap on Esc, Tab and Up sends each; slides and swipes on Esc, Tab, `/` and swipes on Up and PgDn send nothing,
    // but Up held 250 ms acts once before it slides off
    @MainActor
    private func tapsAndSlides(in app: XCUIApplication) -> [String] {
        Self.key("escape", in: app).tap()
        Self.key("tab", in: app).tap()
        Self.key("upArrow", in: app).tap()
        var sent = ["escape/0", "tab/0", "arrow_up/0"]
        Self.expect("bench-keys", reads: sent.joined(separator: " "), in: app)
        shot("01-tap-esc-tab-up")
        Self.slide("escape", to: "slash", in: app)
        Self.slide("tab", to: "control", in: app)
        Self.slide("slash", to: nil, in: app)
        Self.swipeInside("escape", in: app)
        Self.swipeInside("tab", in: app)
        Self.swipeInside("slash", in: app)
        Self.swipeInside("upArrow", in: app)
        Self.swipeInside("pageDown", in: app)
        // held past its hold, an arrow acts once before it slides off
        Self.slide("upArrow", to: nil, in: app)
        Self.key("pipe", in: app).tap()
        sent += ["arrow_up/0"]
        Self.expect("bench-keys", reads: sent.joined(separator: " "), in: app)
        Self.expect("bench-typed", reads: "|", in: app)
        shot("02-slides-and-swipes-send-nothing")
        return sent
    }

    // Ctrl and Alt: a slide off or a swipe in toggles nothing; a tap toggles on the lift, for the next key
    @MainActor
    private func modifiers(after sent: [String], keyboard: XCUIElement, in app: XCUIApplication) -> [String] {
        Self.slide("control", to: "alt", in: app)
        Self.swipeInside("control", in: app)
        Self.slide("alt", to: "control", in: app)
        Self.swipeInside("alt", in: app)
        XCTAssertEqual(Self.key("control", in: app).value as? String, "Off", "a slide or a swipe toggled Ctrl")
        XCTAssertEqual(Self.key("alt", in: app).value as? String, "Off", "a slide or a swipe toggled Alt")
        Self.key("control", in: app).tap()
        XCTAssertEqual(Self.key("control", in: app).value as? String, "On for the next key")
        shot("03-ctrl-on-for-the-next-key")
        // the system keyboard's key reaches the app a moment after its tap: the test waits for it before the next tap
        keyboard.keys["c"].tap()
        Self.expect("bench-keys", reads: (sent + ["key_c/2"]).joined(separator: " "), in: app)
        Self.key("alt", in: app).tap()
        XCTAssertEqual(Self.key("alt", in: app).value as? String, "On for the next key")
        keyboard.keys["x"].tap()
        let after = sent + ["key_c/2", "key_x/4"]
        Self.expect("bench-keys", reads: after.joined(separator: " "), in: app)
        XCTAssertEqual(Self.key("control", in: app).value as? String, "Off")
        XCTAssertEqual(Self.key("alt", in: app).value as? String, "Off")
        shot("04-ctrl-c-alt-x")
        return after
    }

    // an arrow held 250 ms acts once, by its hold; held 800 ms it repeats
    @MainActor
    private func heldArrows(after sent: [String], in app: XCUIApplication) -> [String] {
        Self.point("upArrow", in: app).press(forDuration: Self.holdPress)
        let once = sent + ["arrow_up/0"]
        Self.expect("bench-keys", reads: once.joined(separator: " "), in: app)
        Self.point("downArrow", in: app).press(forDuration: Self.longPress)
        // the bench draws its lines five times a second: the last repeats show a moment after the lift
        _ = XCTWaiter.wait(for: [XCTestExpectation(description: "the last repeats show")], timeout: Self.settle)
        let prefix = once.joined(separator: " ")
        let label = Self.read("bench-keys", in: app) { label in
            label.hasPrefix(prefix) && label.dropFirst(prefix.count).split(separator: " ").count >= Self.leastRepeats
        }
        // what came after the earlier keys: nothing when the earlier keys are not as expected
        let tail = label.flatMap { $0.hasPrefix(prefix) ? $0.dropFirst(prefix.count) : nil } ?? ""
        let held = tail.split(separator: " ").map(String.init)
        XCTAssertGreaterThanOrEqual(held.count, Self.leastRepeats, "Down held 800 ms sent \(held)")
        XCTAssertEqual(Set(held), ["arrow_down/0"], "Down held 800 ms sent \(held)")
        let attachment = XCTAttachment(string: "Down held 800 ms: \(held.count) keys")
        attachment.lifetime = .keepAlways
        add(attachment)
        shot("05-held-arrows")
        return once + held
    }

    // the keyboard's key: a slide off or a swipe in keeps the soft keyboard, a tap closes it. Then a swipe up from the
    // screen's foot over Esc sends nothing (the system may take it and send the app away), and Down at the foot goes
    @MainActor
    private func theKeyboardsKeyAndTheFoot(after sent: [String], keyboard: XCUIElement, in app: XCUIApplication) {
        Self.slide("keyboard", to: "alt", in: app)
        Self.swipeInside("keyboard", in: app)
        XCTAssertEqual(ScreenRead.isShown(keyboard), .holds, "a slide off or a swipe in the keyboard's key closed it")
        Self.key("keyboard", in: app).tap()
        FakeApp.expectGone(keyboard, "the soft keyboard", lasting: Self.wait)
        shot("06-keyboard-closed")
        let escape = Self.point("escape", in: app).screenPoint
        let foot = app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 1))
        let edge = foot.withOffset(CGVector(dx: escape.x, dy: -1))
        let terminal = Self.point(nil, in: app)
        edge.press(forDuration: Self.quickPress, thenDragTo: terminal, withVelocity: .fast, thenHoldForDuration: 0)
        // the system may take the swipe and show the Home Screen: then the app comes back to the front
        var away = false
        let end = Date().addingTimeInterval(Self.awayWait)
        while !away, Date() < end {
            away = app.state != .runningForeground
            _ = XCTWaiter.wait(for: [XCTestExpectation(description: "a beat")], timeout: Self.readBeat)
        }
        shot("07-after-the-swipe-from-the-foot")
        let note = XCTAttachment(string: away ? "the system took the swipe: the Home Screen showed" : "the app kept it")
        note.lifetime = .keepAlways
        add(note)
        if away {
            app.activate()
            XCTAssertTrue(app.wait(for: .runningForeground, timeout: Self.wait), "the app did not come back")
        }
        // back from the Home Screen the terminal may not have the focus: a tap on it gives it back, with the keys
        let down = Self.key("downArrow", in: app)
        let back = ScreenRead.Wait(reads: 1, lasting: Self.introductionWait).poll { ScreenRead.isShown(down) }
        if !back.held {
            app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        }
        FakeApp.expect(down, "the extra keys after a swipe from the foot")
        // the swipe sent nothing
        Self.expect("bench-keys", reads: sent.joined(separator: " "), in: app)
        shot("08-back-in-the-app")
        down.tap()
        Self.expect("bench-keys", reads: (sent + ["arrow_down/0"]).joined(separator: " "), in: app)
        shot("09-down-at-the-foot")
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
        let bigger = Self.read("bench-grid", in: app) { $0 != grid }
        XCTAssertNotNil(bigger, "no read found bench-grid after ⌘=")
        XCTAssertNotEqual(bigger ?? grid, grid, "⌘= made the text no bigger")
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
