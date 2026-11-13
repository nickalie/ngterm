import XCTest

/// Duplicate session on the fake core's `split`: from a session row's context menu and from the session view's More,
/// the copy (`session 1`, whose pane another device leads as it attaches) opens at once in this device's control, its
/// pane's screen showing with no Take control cover; a Duplicate that fails says why and opens nothing.
internal final class DuplicateSessionTests: XCTestCase {
    // what the fake world's first Duplicate fails with, when asked (FakeSessionWorld.duplicateRefusal)
    private static let refusal = "split could not copy work"

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

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
