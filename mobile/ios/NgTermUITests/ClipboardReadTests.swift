import XCTest

/// A program's clipboard read in the app on the fake core (mobile/android's `ClipboardReadPromptTest`): the sheet asks
/// over whatever shows, passes the accessibility audit, and goes once answered, or swiped away (which denies).
internal final class ClipboardReadTests: XCTestCase {
    @MainActor
    private func launch() -> XCUIApplication {
        let app = FakeApp.launch(withKey: true, arguments: ["-ngterm-fake-clipboard-read"], for: self)
        FakeApp.expect(app.staticTexts["Let docs-site read your clipboard?"], "clipboard question")
        return app
    }

    @MainActor
    internal func testDenyAnswersAndTheSheetGoes() throws {
        let app = launch()
        XCTAssertTrue(FakeApp.element(containing: "A program in docs-site on build-01", in: app).exists)
        XCTAssertTrue(app.buttons["clipboard-always"].label.contains("build-01"))
        try FakeApp.auditSheet(app)
        app.buttons["clipboard-deny"].tap()
        XCTAssertTrue(app.staticTexts["Let docs-site read your clipboard?"].waitForNonExistence(timeout: 10))
    }

    @MainActor
    internal func testSwipingTheSheetAwayDenies() {
        let app = launch()
        // the sheet pulled down from its heading to the screen's foot
        let heading = app.staticTexts["Let docs-site read your clipboard?"]
        let foot = app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.98))
        heading.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: foot, withVelocity: .fast, thenHoldForDuration: 0)
        XCTAssertTrue(app.staticTexts["Let docs-site read your clipboard?"].waitForNonExistence(timeout: 10))
        // the read was answered: the question doesn't come back
        XCTAssertFalse(app.staticTexts["Let docs-site read your clipboard?"].waitForExistence(timeout: 2))
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
