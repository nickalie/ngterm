import XCTest

/// The session view on the fake core's `split`, whose session `work` has two panes side by side, each naming
/// itself on its first line ("P1 ready"): a phone shows one at a time, a tablet the split.
///
/// Switching panes, then turning the device (the layout made anew) and back, keeps the selected pane; the session view
/// is audited.
internal final class SessionViewTests: XCTestCase {
    @MainActor private static var isPad: Bool {
        UIDevice.current.userInterfaceIdiom == .pad
    }

    // the terminal whose screen shows `text` (VoiceOver's value of it)
    @MainActor
    private static func screen(showing text: String, in app: XCUIApplication) -> XCUIElement {
        app.textViews.matching(NSPredicate(format: "value CONTAINS %@", text)).firstMatch
    }

    // the app on `work`'s session view, its first pane selected
    @MainActor
    private func openSession() -> XCUIApplication {
        let app = FakeApp.launch(withKey: true, arguments: ["-ngterm-fake-session"], for: self)
        FakeApp.showMachines(in: app)
        FakeApp.expectRow("split", reads: "Connected", in: app)
        app.descendants(matching: .any)["machine-row-split"].firstMatch.tap()
        let row = app.descendants(matching: .any)["session-work"].firstMatch
        FakeApp.expect(row, "the session's row")
        row.tap()
        FakeApp.expect(app.descendants(matching: .any)["session-title"].firstMatch, "the session view")
        FakeApp.expect(Self.screen(showing: "P1 ready", in: app), "the first pane's screen")
        return app
    }

    @MainActor
    private func expectSelected(_ number: Int, in app: XCUIApplication) {
        let tab = app.buttons["pane-\(number)"]
        FakeApp.expect(tab, "pane \(number)'s tab")
        let selected = NSPredicate(format: "isSelected == true")
        let waited = XCTNSPredicateExpectation(predicate: selected, object: tab)
        XCTAssertEqual(XCTWaiter.wait(for: [waited], timeout: 10), .completed, "pane \(number) isn't selected")
    }

    @MainActor
    internal func testSwitchingPanesAndTurningKeepTheSelectedPane() throws {
        let app = openSession()
        expectSelected(1, in: app)
        try FakeApp.audit(app)

        app.buttons["pane-2"].tap()
        expectSelected(2, in: app)
        let second = Self.screen(showing: "P2 ready", in: app)
        FakeApp.expect(second, "the second pane's screen")
        if Self.isPad {
            XCTAssertTrue(Self.screen(showing: "P1 ready", in: app).exists, "the split shows both panes")
            let sidebar = app.descendants(matching: .any)["sidebar-session-work"].firstMatch
            XCTAssertTrue(sidebar.exists, "the machines' column lists the attached session")
        } else {
            XCTAssertFalse(Self.screen(showing: "P1 ready", in: app).exists, "a phone shows one pane")
        }

        // the layout made anew on its side and upright again keeps the selection
        let turned: UIDeviceOrientation = Self.isPad ? .portrait : .landscapeLeft
        let back: UIDeviceOrientation = Self.isPad ? .landscapeLeft : .portrait
        XCUIDevice.shared.orientation = turned
        expectSelected(2, in: app)
        FakeApp.expect(second, "the second pane's screen, turned")
        XCUIDevice.shared.orientation = back
        expectSelected(2, in: app)
        FakeApp.expect(second, "the second pane's screen, turned back")
        try FakeApp.audit(app)
    }

    // a phone's machines list is its view of every attached session (Android's Sessions destination), a tablet's
    // sidebar beside the session
    @MainActor
    internal func testTheMachinesListTheAttachedSessionAndOpenIt() throws {
        let app = openSession()
        let add = app.buttons["add-machine"]
        // a phone goes back from the session and the machine's sessions to the machines
        for _ in 0..<2 where !add.exists {
            app.navigationBars.buttons.element(boundBy: 0).tap()
        }
        FakeApp.expect(add, "the machines")
        let row = app.descendants(matching: .any)["sidebar-session-work"].firstMatch
        FakeApp.expect(row, "the attached session under its machine")
        if !Self.isPad {
            XCTAssertFalse(app.descendants(matching: .any)["session-title"].firstMatch.exists, "the session left")
            try FakeApp.audit(app)
        }
        row.tap()
        FakeApp.expect(app.descendants(matching: .any)["session-title"].firstMatch, "the session view again")
        FakeApp.expect(Self.screen(showing: "P1 ready", in: app), "the first pane's screen")
    }

    @MainActor
    internal func testDetachClosesTheSessionView() {
        let app = openSession()
        app.buttons["session-menu"].tap()
        let detach = app.buttons["session-detach"]
        FakeApp.expect(detach, "Detach in the session's menu")
        detach.tap()
        let title = app.descendants(matching: .any)["session-title"].firstMatch
        let gone = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: title)
        XCTAssertEqual(XCTWaiter.wait(for: [gone], timeout: 10), .completed, "the session view stayed")
        FakeApp.expect(app.descendants(matching: .any)["session-work"].firstMatch, "the session's row again")
    }

    // the fake core lists the session's machine without it before End answers: the view closes all the same, rather
    // than staying on the session gone
    @MainActor
    internal func testEndClosesTheSessionView() throws {
        let app = openSession()
        app.buttons["session-menu"].tap()
        let end = app.buttons["session-end"]
        FakeApp.expect(end, "End session in the session's menu")
        end.tap()
        FakeApp.expect(FakeApp.element(containing: "End work?", in: app), "End session confirmation")
        let confirm = app.buttons.matching(identifier: "End session").allElementsBoundByIndex.first(where: \.isHittable)
        try XCTUnwrap(confirm, "no End session in the confirmation").tap()
        let absent = NSPredicate(format: "exists == false")
        let title = app.descendants(matching: .any)["session-title"].firstMatch
        let live = XCTNSPredicateExpectation(predicate: absent, object: title)
        XCTAssertEqual(XCTWaiter.wait(for: [live], timeout: 10), .completed, "the session view stayed")
        FakeApp.expect(FakeApp.element(containing: "No sessions on split", in: app), "the machine's empty list")
        let goneShown = FakeApp.element(containing: "work is gone", in: app)
        let gone = XCTNSPredicateExpectation(predicate: absent, object: goneShown)
        XCTAssertEqual(XCTWaiter.wait(for: [gone], timeout: 10), .completed, "the view stayed on the session gone")
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
