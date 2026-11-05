import XCTest

/// Asks in the app on the fake core (mobile/android's `AskFlowTest`).
///
/// The origin of `work` on `split` asks whether to deploy as the session is attached. Its sheet shows over the session
/// view, passes the accessibility audit, and goes once answered; the origin then says what it does with the answer as
/// the session's context, which proves the answer reached the core. Swiping the sheet away answers Escape. The question
/// about notifying shows once, as the first session is attached.
internal final class AskTests: XCTestCase {
    private static let question = "Deploy api to production?"

    // the app on `work`'s session view, with `arguments` besides the fake session's
    @MainActor
    private func openSession(_ arguments: [String]) -> XCUIApplication {
        let app = FakeApp.launch(withKey: true, arguments: ["-ngterm-fake-session"] + arguments, for: self)
        FakeApp.showMachines(in: app)
        FakeApp.expectRow("split", reads: "Connected", in: app)
        app.descendants(matching: .any)["machine-row-split"].firstMatch.tap()
        let row = app.descendants(matching: .any)["session-work"].firstMatch
        FakeApp.expect(row, "the session's row")
        row.tap()
        return app
    }

    @MainActor
    internal func testAnsweringTheAskInTheApp() throws {
        let app = openSession(["-ngterm-fake-ask"])
        FakeApp.expect(app.staticTexts[Self.question], "the ask's sheet")
        XCTAssertTrue(app.staticTexts["The tests passed on main."].exists)
        try FakeApp.auditSheet(app)
        app.buttons["ask-button-deploy"].tap()
        XCTAssertTrue(app.staticTexts[Self.question].waitForNonExistence(timeout: 10))
        FakeApp.expect(FakeApp.element(containing: "Deploying to production", in: app), "the origin acting on it")
    }

    @MainActor
    internal func testSwipingTheSheetAwayEscapes() {
        let app = openSession(["-ngterm-fake-ask"])
        let heading = app.staticTexts[Self.question]
        FakeApp.expect(heading, "the ask's sheet")
        let foot = app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.98))
        heading.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: foot, withVelocity: .fast, thenHoldForDuration: 0)
        XCTAssertTrue(heading.waitForNonExistence(timeout: 10))
        FakeApp.expect(FakeApp.element(containing: "Deploy called off", in: app), "the origin escaping")
    }

    @MainActor
    internal func testTheQuestionAboutNotifyingShowsOnceAsASessionIsAttached() throws {
        let app = openSession(["-ngterm-fake-notification-question"])
        let title = app.staticTexts["Get told when an agent needs you"]
        FakeApp.expect(title, "the question about notifying")
        try FakeApp.auditSheet(app)
        app.buttons["notifications-later"].tap()
        XCTAssertTrue(title.waitForNonExistence(timeout: 10))
        FakeApp.expect(app.descendants(matching: .any)["session-title"].firstMatch, "the session view")
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
