import XCTest

/// A machine's sessions on the fake core: New session in one tap and named, each landing in its session, and End.
internal final class SessionsTests: XCTestCase {
    @MainActor
    internal func testNewSessionLandsInItThenEndsFromTheList() throws {
        let app = FakeApp.launch(withKey: true, for: self)
        // a host the fake world connects at once, with no sessions
        try FakeApp.addMachine(host: "box.lan", user: "nick", in: app, audit: false)
        FakeApp.expectRow("box.lan", reads: "Connected", in: app)
        app.descendants(matching: .any)["machine-row-box.lan"].firstMatch.tap()

        let empty = FakeApp.element(containing: "No sessions on box.lan", in: app)
        FakeApp.expect(empty, "empty state")
        try FakeApp.audit(app)

        let newSession = app.buttons["new-session"]
        newSession.tap()
        let title = app.descendants(matching: .any)["session-title"].firstMatch
        FakeApp.expect(title, "the new session's view")
        FakeApp.expect(FakeApp.element(containing: "session 1", in: app), "the session the machine named")
        let name = app.textFields["session-name"]
        XCTAssertFalse(name.exists, "a sheet came up on the way")
        endShownSession(named: "session 1", in: app)
        FakeApp.expect(empty, "empty state after the End")

        newSession.press(forDuration: 1)
        let named = app.buttons["new-named-session"]
        FakeApp.expect(named, "New named session in the button's menu")
        named.tap()
        FakeApp.expect(name, "New session sheet")
        try FakeApp.auditSheet(app)
        name.tap()
        name.typeText("api")
        app.buttons["create-session"].tap()
        FakeApp.expect(title, "the named session's view")
        FakeApp.expect(FakeApp.element(containing: "api", in: app), "the named session")
        XCTAssertFalse(name.exists)

        let row = app.descendants(matching: .any)["session-api"].firstMatch
        if !row.exists {
            app.navigationBars.buttons.element(boundBy: 0).tap()
        }
        FakeApp.expect(row, "the new session's row")
        try FakeApp.audit(app)

        row.swipeLeft()
        let swiped = app.buttons["End session"].firstMatch
        FakeApp.expect(swiped, "End session swipe action")
        swiped.tap()
        FakeApp.expect(FakeApp.element(containing: "End api?", in: app), "End session confirmation")
        confirmEnd(in: app)

        FakeApp.expect(empty, "empty state after the End")
        XCTAssertFalse(row.exists)
    }

    @MainActor
    private func endShownSession(named name: String, in app: XCUIApplication) {
        app.buttons["session-menu"].tap()
        let end = app.buttons["session-end"]
        FakeApp.expect(end, "End session in the session's menu")
        end.tap()
        FakeApp.expect(FakeApp.element(containing: "End \(name)?", in: app), "End session confirmation")
        confirmEnd(in: app)
        let title = app.descendants(matching: .any)["session-title"].firstMatch
        let gone = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: title)
        XCTAssertEqual(XCTWaiter.wait(for: [gone], timeout: 10), .completed, "the session view stayed")
    }

    @MainActor
    private func confirmEnd(in app: XCUIApplication) {
        let confirm = app.buttons.matching(identifier: "End session").allElementsBoundByIndex.first(where: \.isHittable)
        guard let confirm else {
            XCTFail("no End session in the confirmation")
            return
        }
        confirm.tap()
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
