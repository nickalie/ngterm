import XCTest

/// Duplicate session, from a session's row and from its session view.
///
/// On the fake core's `split`, from a session row's context menu and from the session view's More, the copy
/// (`session 1`, whose pane another device leads as it attaches) opens at once in this device's control, its pane's
/// screen showing with no Take control cover; a Duplicate that fails says why and opens nothing. While one runs (the
/// fake world holds it a while, when asked), its source says "Duplicating", and New session and every other Duplicate
/// take no tap. In an iPad's session window the copy opens in a window of its own. An agterm Mac's session duplicates
/// as an ngterm machine's does.
internal final class DuplicateSessionTests: XCTestCase {
    // what the fake world's first Duplicate fails with, when asked (FakeSessionWorld.duplicateRefusal)
    private static let refusal = "split could not copy work"
    // how long a press on a row holds to bring up its context menu
    private static let longPress: TimeInterval = 1
    // where a tap closes the menu that is up: low on the dimmed screen, away from the menu and the home indicator
    private static let offMenu = CGVector(dx: 0.5, dy: 0.9)
    // how long windows take to open or close, and the most session windows a test opens
    private static let windowWait: TimeInterval = 10
    private static let maxWindows = 3

    @MainActor
    private static func any(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any)[identifier].firstMatch
    }

    // the row of the session `name` whose label holds `text`
    @MainActor
    private static func row(_ name: String, reading text: String, in app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier == %@ AND label CONTAINS %@", "session-\(name)", text))
            .firstMatch
    }

    // the context menu's Duplicate session of the session `name`'s row
    @MainActor
    private static func rowsDuplicate(_ name: String, in app: XCUIApplication) -> XCUIElement {
        any("session-\(name)", in: app).press(forDuration: longPress)
        let duplicate = app.buttons["Duplicate session"]
        FakeApp.expect(duplicate, "Duplicate session in \(name)'s context menu")
        return duplicate
    }

    // closes the menu that is up with a tap beside it
    @MainActor
    private static func closeMenu(in app: XCUIApplication) {
        app.windows.firstMatch.coordinate(withNormalizedOffset: offMenu).tap()
    }

    // the session view's title naming `name`
    @MainActor
    private static func title(naming name: String, in app: XCUIElement) -> XCUIElement {
        app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier == 'session-title' AND label CONTAINS %@", name))
            .firstMatch
    }

    // the terminal whose screen shows `text`: only while no cover hides the pane
    @MainActor
    private static func screen(showing text: String, in app: XCUIElement) -> XCUIElement {
        app.textViews.matching(NSPredicate(format: "value CONTAINS %@", text)).firstMatch
    }

    // the copy is open, its pane in this device's control: its screen shows, and no Take control cover came up
    @MainActor
    private static func expectCopyInControl(in app: XCUIApplication) {
        FakeApp.expect(title(naming: "session 1", in: app), "the copy's session view")
        FakeApp.expect(screen(showing: "P1 ready", in: app), "the copy's pane in control")
        XCTAssertFalse(app.buttons["Take control"].exists, "the copy offered Take control")
        XCTAssertFalse(FakeApp.element(containing: "Another device is driving", in: app).exists)
    }

    // waits until the app has `count` windows
    @MainActor
    private static func expectWindows(_ count: Int, in app: XCUIApplication) {
        let windows = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "count == %d", count),
            object: app.windows
        )
        XCTAssertEqual(XCTWaiter.wait(for: [windows], timeout: windowWait), .completed, "not \(count) windows")
    }

    // closes the session windows: Detach in the More of a session window in front, or, with the app's own window in
    // front, Detach in the context menu of a row whose session a window shows
    @MainActor
    private static func closeSessionWindows(in app: XCUIApplication) {
        for _ in 0..<maxWindows {
            let count = app.windows.count
            guard count > 1 else {
                return
            }
            let front = app.windows.element(boundBy: 0)
            if front.buttons["add-machine"].exists {
                guard detachARow(of: ["session 1", "work"], in: app) else {
                    return
                }
            } else {
                front.buttons["session-menu"].tap()
                let detach = app.buttons["session-detach"]
                guard detach.waitForExistence(timeout: windowWait) else {
                    return
                }
                detach.tap()
            }
            expectWindows(count - 1, in: app)
        }
    }

    // detaches the first of the sessions `names` whose row's context menu offers Detach; false when none does
    @MainActor
    private static func detachARow(of names: [String], in app: XCUIApplication) -> Bool {
        for name in names where any("session-\(name)", in: app).exists {
            any("session-\(name)", in: app).press(forDuration: longPress)
            let detach = app.buttons["Detach"]
            if detach.waitForExistence(timeout: windowWait) {
                detach.tap()
                return true
            }
            closeMenu(in: app)
        }
        return false
    }

    // the app on `split`'s sessions, as `arguments` ask
    @MainActor
    private func openMachine(arguments: [String] = []) -> XCUIApplication {
        let app = FakeApp.launch(withKey: true, arguments: ["-ngterm-fake-session"] + arguments, for: self)
        FakeApp.showMachines(in: app)
        FakeApp.expectRow("split", reads: "Connected", in: app)
        app.descendants(matching: .any)["machine-row-split"].firstMatch.tap()
        FakeApp.expect(app.descendants(matching: .any)["session-work"].firstMatch, "work's row")
        return app
    }

    @MainActor
    private func duplicateFromMenu(in app: XCUIApplication) {
        app.buttons["session-menu"].tap()
        let duplicate = app.buttons["session-duplicate"]
        FakeApp.expect(duplicate, "Duplicate session in the session's More")
        duplicate.tap()
    }

    @MainActor
    internal func testARowsDuplicateOpensTheCopyInControl() throws {
        let app = openMachine()
        app.descendants(matching: .any)["session-work"].firstMatch.press(forDuration: 1)
        let duplicate = app.buttons["Duplicate session"]
        FakeApp.expect(duplicate, "Duplicate session in the row's context menu")
        duplicate.tap()
        Self.expectCopyInControl(in: app)
        try FakeApp.audit(app)
    }

    @MainActor
    internal func testTheSessionViewsDuplicateSaysWhyItFailedThenOpensTheCopy() {
        let app = openMachine(arguments: ["-ngterm-fake-duplicate-fails"])
        app.descendants(matching: .any)["session-work"].firstMatch.tap()
        FakeApp.expect(Self.title(naming: "work", in: app), "work's session view")

        duplicateFromMenu(in: app)
        FakeApp.expect(FakeApp.element(containing: Self.refusal, in: app), "the failed Duplicate's banner")
        XCTAssertTrue(Self.title(naming: "work", in: app).exists, "a failed Duplicate left the session")
        XCTAssertFalse(Self.title(naming: "session 1", in: app).exists, "a failed Duplicate opened a copy")

        duplicateFromMenu(in: app)
        Self.expectCopyInControl(in: app)
    }

    @MainActor
    internal func testWhileADuplicateRunsItsSourceSaysSoAndNothingElseCreates() {
        let app = openMachine(arguments: ["-ngterm-fake-companion", "-ngterm-fake-duplicate-slow"])
        Self.rowsDuplicate("work", in: app).tap()
        FakeApp.expect(Self.row("work", reading: "Duplicating", in: app), "work's Duplicating mark")
        XCTAssertFalse(app.buttons["new-session"].isEnabled, "New session took a tap while a Duplicate ran")
        let other = Self.rowsDuplicate("notes", in: app)
        XCTAssertFalse(other.isEnabled, "notes's Duplicate took a tap while work's ran")
        Self.closeMenu(in: app)
        Self.expectCopyInControl(in: app)
        XCTAssertFalse(Self.row("work", reading: "Duplicating", in: app).exists, "work still says Duplicating")

        // the copy's own Duplicate, from its session view: its title says so, and its More's Duplicate takes no tap
        duplicateFromMenu(in: app)
        FakeApp.expect(Self.title(naming: "Duplicating", in: app), "the title's Duplicating mark")
        app.buttons["session-menu"].tap()
        let again = app.buttons["session-duplicate"]
        FakeApp.expect(again, "Duplicate session in the session's More")
        XCTAssertFalse(again.isEnabled, "the More's Duplicate took a tap while one ran")
        Self.closeMenu(in: app)
        // an iPad shows the sessions beside the session
        let newSession = app.buttons["new-session"]
        if newSession.exists {
            XCTAssertFalse(newSession.isEnabled, "New session beside the session took a tap while a Duplicate ran")
        }
        FakeApp.expect(Self.title(naming: "session 2", in: app), "the second copy's session view")
        XCTAssertFalse(Self.title(naming: "Duplicating", in: app).exists, "the copy's title still says Duplicating")
    }

    // an iPad's session window: the copy opens in a window of its own, in front (the app's first window), and the
    // source's window stays behind it, showing the source; the test closes the windows it opened (Detach), or the
    // system would bring them back on the next launch
    @MainActor
    internal func testInASessionWindowTheCopyOpensInAWindowOfItsOwn() throws {
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .pad, "only an iPad opens session windows")
        let app = openMachine()
        addTeardownBlock { @MainActor in
            Self.closeSessionWindows(in: app)
        }
        Self.any("session-work", in: app).tap()
        FakeApp.expect(Self.title(naming: "work", in: app), "work's session view")
        app.buttons["session-menu"].tap()
        let newWindow = app.buttons["session-new-window"]
        FakeApp.expect(newWindow, "Open in New Window in the session's More")
        newWindow.tap()
        Self.expectWindows(2, in: app)
        let front = app.windows.element(boundBy: 0)
        FakeApp.expect(Self.title(naming: "work", in: front), "work's window in front")
        XCTAssertFalse(front.buttons["add-machine"].exists, "work's window shows the machines")

        front.buttons["session-menu"].tap()
        let duplicate = app.buttons["session-duplicate"]
        FakeApp.expect(duplicate, "Duplicate session in work's window's More")
        duplicate.tap()
        Self.expectWindows(3, in: app)
        let copy = app.windows.element(boundBy: 0)
        FakeApp.expect(Self.title(naming: "session 1", in: copy), "the copy's window in front")
        FakeApp.expect(Self.screen(showing: "P1 ready", in: copy), "the copy's pane in control")
        XCTAssertFalse(copy.buttons["Take control"].exists, "the copy offered Take control")
        XCTAssertFalse(copy.buttons["add-machine"].exists, "the copy opened beside the machines")
        let behind = app.windows.allElementsBoundByIndex.dropFirst()
        XCTAssertTrue(
            behind.contains { Self.title(naming: "work", in: $0).exists },
            "work's window went: \(app.debugDescription)"
        )
    }

    // an agterm Mac's session: its row's context menu offers Duplicate session, and the copy opens, saying agterm
    @MainActor
    internal func testAnAgtermSessionDuplicatesAsAnNgtermOne() {
        let app = FakeApp.launch(withKey: true, arguments: ["-ngterm-fake-agterm"], for: self)
        FakeApp.showMachines(in: app)
        FakeApp.expectRow("studio", reads: "Connected", in: app)
        Self.any("machine-row-studio", in: app).tap()
        FakeApp.expect(Self.any("session-notes", in: app), "notes's row")
        let duplicate = Self.rowsDuplicate("notes", in: app)
        XCTAssertTrue(duplicate.isEnabled, "agterm's Duplicate takes no tap")
        duplicate.tap()
        let copy = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier == 'session-title' AND label CONTAINS 'session 1'"))
            .matching(NSPredicate(format: "label CONTAINS 'agterm'"))
            .firstMatch
        FakeApp.expect(copy, "the copy's session view with its agterm badge")
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
