import XCTest

/// A machine's sessions on the fake core.
///
/// New session, the session in the list, End session, and the list without it; the empty state, the sheet and the
/// list are audited.
internal final class SessionsTests: XCTestCase {
    @MainActor
    internal func testCreateAppearsEndDisappears() throws {
        let app = FakeApp.launch(withKey: true, for: self)
        // a host the fake world connects at once, with no sessions
        try FakeApp.addMachine(host: "box.lan", user: "nick", in: app, audit: false)
        FakeApp.expectRow("box.lan", reads: "Connected", in: app)
        app.descendants(matching: .any)["machine-box.lan"].firstMatch.tap()

        let empty = FakeApp.element(containing: "No sessions on box.lan", in: app)
        FakeApp.expect(empty, "empty state")
        try FakeApp.audit(app)
        app.buttons["new-session"].tap()

        let name = app.textFields["session-name"]
        FakeApp.expect(name, "New session sheet")
        try FakeApp.auditSheet(app)
        name.tap()
        name.typeText("api")
        app.buttons["create-session"].tap()

        let row = app.descendants(matching: .any)["session-api"].firstMatch
        FakeApp.expect(row, "the new session's row")
        XCTAssertFalse(name.exists)
        try FakeApp.audit(app)

        row.swipeLeft()
        let swiped = app.buttons["End session"].firstMatch
        FakeApp.expect(swiped, "End session swipe action")
        swiped.tap()
        FakeApp.expect(FakeApp.element(containing: "End api?", in: app), "End session confirmation")
        // the swipe action has gone by now: the one End session left on screen is the confirmation's
        let confirm = app.buttons.matching(identifier: "End session").allElementsBoundByIndex.first(where: \.isHittable)
        try XCTUnwrap(confirm, "no End session in the confirmation").tap()

        FakeApp.expect(empty, "empty state after the End")
        XCTAssertFalse(row.exists)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
