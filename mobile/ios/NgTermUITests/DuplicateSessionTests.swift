import XCTest

/// Duplicate session on the fake core's `split`: from a session row's context menu and from the session view's More,
/// the copy (`session 1`, whose pane another device leads as it attaches) opens at once in this device's control, its
/// pane's screen showing with no Take control cover; a Duplicate that fails says why and opens nothing. While one runs
/// (the fake world holds it a while, when asked), its source says "Duplicating", and New session and every other
/// Duplicate take no tap. In an iPad's session window the copy opens in a window of its own. An agterm Mac's session
/// duplicates as an ngterm machine's does.
internal final class DuplicateSessionTests: XCTestCase {
    // what the fake world's first Duplicate fails with, when asked (FakeSessionWorld.duplicateRefusal)
    private static let refusal = "split could not copy work"
    // how long a press on a row holds to bring up its context menu
    private static let longPress: TimeInterval = 1
    // where a tap closes the menu that is up: low on the dimmed screen, away from the menu and the home indicator
    private static let offMenu = CGVector(dx: 0.5, dy: 0.9)

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
    private static func title(naming name: String, in app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier == 'session-title' AND label CONTAINS %@", name))
            .firstMatch
    }

    // the terminal whose screen shows `text`: only while no cover hides the pane
    @MainActor
    private static func screen(showing text: String, in app: XCUIApplication) -> XCUIElement {
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

    // an iPad's session window: the copy opens in a window of its own, and the source's window stays, which shows again
    // once the copy's window closes (Detach)
    @MainActor
    internal func testInASessionWindowTheCopyOpensInAWindowOfItsOwn() throws {
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .pad, "only an iPad opens session windows")
        let app = openMachine()
        Self.any("session-work", in: app).tap()
        FakeApp.expect(Self.title(naming: "work", in: app), "work's session view")
        app.buttons["session-menu"].tap()
        let newWindow = app.buttons["session-new-window"]
        FakeApp.expect(newWindow, "Open in New Window in the session's More")
        newWindow.tap()
        // work's own window: no machines beside it
        FakeApp.expect(Self.title(naming: "work", in: app), "work's window")
        XCTAssertTrue(app.buttons["add-machine"].waitForNonExistence(timeout: 10), "work's window shows the machines")
        attach(app, "work's window")

        duplicateFromMenu(in: app)
        Self.expectCopyInControl(in: app)
        XCTAssertFalse(app.buttons["add-machine"].exists, "the copy opened beside the machines")
        attach(app, "the copy's window")

        app.buttons["session-menu"].tap()
        let detach = app.buttons["session-detach"]
        FakeApp.expect(detach, "Detach in the copy's More")
        detach.tap()
        attach(app, "after the copy's Detach")
        FakeApp.expect(Self.title(naming: "work", in: app), "work's window, once the copy's closed")
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

    // keeps the screen and its element tree in the result, named `what`
    @MainActor
    private func attach(_ app: XCUIApplication, _ what: String) {
        XCTContext.runActivity(named: what) { activity in
            let shot = XCTAttachment(screenshot: app.screenshot())
            shot.lifetime = .keepAlways
            activity.add(shot)
            let tree = XCTAttachment(string: app.debugDescription)
            tree.lifetime = .keepAlways
            activity.add(tree)
        }
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
