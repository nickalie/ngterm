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

    // the attached session `id`'s row in the machines' column
    @MainActor
    private static func sidebarRow(_ id: String, in app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any)["sidebar-session-\(id)"].firstMatch
    }

    // opens the session `id` from its machine's sessions, and waits for its pane's screen `text`
    @MainActor
    private static func open(_ id: String, showing text: String, in app: XCUIApplication) {
        let row = app.descendants(matching: .any)["session-\(id)"].firstMatch
        FakeApp.expect(row, "\(id)'s row")
        row.tap()
        FakeApp.expect(screen(showing: text, in: app), "\(id)'s pane")
    }

    // a phone goes back from the session and the machine's sessions to the machines; a tablet shows them beside it
    @MainActor
    private static func backToMachines(in app: XCUIApplication) {
        let add = app.buttons["add-machine"]
        for _ in 0..<2 where !add.exists {
            app.navigationBars.buttons.element(boundBy: 0).tap()
        }
        FakeApp.expect(add, "the machines")
    }

    // types ⌘⌥ with `key` and waits for the session whose pane's screen shows `text`
    @MainActor
    private static func moveSession(_ key: String, to text: String, in app: XCUIApplication) {
        app.typeKey(key, modifierFlags: [.command, .option])
        FakeApp.expect(screen(showing: text, in: app), "the session showing \(text) after the move")
    }

    @MainActor
    private static func keepScreenshot(of app: XCUIApplication, named name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        XCTContext.runActivity(named: name) { activity in
            activity.add(shot)
        }
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
            // a tab in the split shows its pane alone; the shown pane's tab brings the split back, still selected
            XCTAssertFalse(Self.screen(showing: "P1 ready", in: app).exists, "a tab shows its pane alone")
            app.buttons["pane-2"].tap()
            expectSelected(2, in: app)
            FakeApp.expect(Self.screen(showing: "P1 ready", in: app), "the split shows both panes again")
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

    // the attached sessions under their machine are a tree, as the machine's own list: a companion right under its
    // parent, saying whose it is
    @MainActor
    internal func testTheMachinesNestAnAttachedCompanionUnderItsParent() throws {
        let arguments = ["-ngterm-fake-session", "-ngterm-fake-companion"]
        let app = FakeApp.launch(withKey: true, arguments: arguments, for: self)
        FakeApp.showMachines(in: app)
        FakeApp.expectRow("split", reads: "Connected", in: app)
        app.descendants(matching: .any)["machine-row-split"].firstMatch.tap()
        let companion = app.descendants(matching: .any)["session-review"].firstMatch
        FakeApp.expect(companion, "the companion's row")
        companion.tap()
        FakeApp.expect(Self.screen(showing: "R1 ready", in: app), "the companion's pane")
        // a phone goes back from the session to the machine's sessions; a tablet shows them beside it
        if !Self.isPad {
            app.navigationBars.buttons.element(boundBy: 0).tap()
        }
        let parent = app.descendants(matching: .any)["session-work"].firstMatch
        FakeApp.expect(parent, "the parent's row")
        parent.tap()
        FakeApp.expect(Self.screen(showing: "P1 ready", in: app), "the parent's first pane")

        let add = app.buttons["add-machine"]
        for _ in 0..<2 where !add.exists {
            app.navigationBars.buttons.element(boundBy: 0).tap()
        }
        FakeApp.expect(add, "the machines")
        let parentRow = app.descendants(matching: .any)["sidebar-session-work"].firstMatch
        let companionRow = app.descendants(matching: .any)["sidebar-session-review"].firstMatch
        FakeApp.expect(companionRow, "the attached companion under its machine")
        XCTAssertTrue(companionRow.label.contains("Companion of work"), "the companion says whose it is")
        XCTAssertGreaterThan(companionRow.frame.minY, parentRow.frame.minY, "the companion comes under its parent")
        if !Self.isPad {
            try FakeApp.audit(app)
        }
        companionRow.tap()
        FakeApp.expect(Self.screen(showing: "R1 ready", in: app), "the companion's pane again")
    }

    // `split` lists work, notes and review (work's companion), the core's order: parents first, companions after.
    // Attached alone, the companion keeps "Companion of work" at the sessions' indent; with its parent attached, the
    // column and the session moves (⌘⌥↓ and ⌘⌥↑ on a hardware keyboard) go work, review, notes, not the listing's order
    @MainActor
    internal func testAnOrphanCompanionStandsAloneAndTheSessionMovesFollowTheTree() throws {
        let arguments = ["-ngterm-fake-session", "-ngterm-fake-companion"]
        let app = FakeApp.launch(withKey: true, arguments: arguments, for: self)
        FakeApp.showMachines(in: app)
        FakeApp.expectRow("split", reads: "Connected", in: app)
        app.descendants(matching: .any)["machine-row-split"].firstMatch.tap()
        Self.open("review", showing: "R1 ready", in: app)
        Self.backToMachines(in: app)
        let companionRow = Self.sidebarRow("review", in: app)
        FakeApp.expect(companionRow, "the attached companion under its machine")
        XCTAssertTrue(companionRow.label.contains("Companion of work"), "the orphan says whose it is")
        XCTAssertFalse(Self.sidebarRow("work", in: app).exists, "the parent isn't attached")
        Self.keepScreenshot(of: app, named: "orphan companion at the sessions' indent")

        // a phone opens the machine's sessions again; a tablet shows them beside the session
        if !Self.isPad {
            app.descendants(matching: .any)["machine-row-split"].firstMatch.tap()
        }
        Self.open("notes", showing: "N1 ready", in: app)
        if !Self.isPad {
            app.navigationBars.buttons.element(boundBy: 0).tap()
        }
        Self.open("work", showing: "P1 ready", in: app)
        Self.backToMachines(in: app)
        let rows = ["work", "review", "notes"].map { Self.sidebarRow($0, in: app) }
        for row in rows {
            FakeApp.expect(row, "\(row.identifier) under its machine")
        }
        XCTAssertLessThan(rows[0].frame.minY, rows[1].frame.minY, "the companion comes under its parent")
        XCTAssertLessThan(rows[1].frame.minY, rows[2].frame.minY, "the next family comes after the companion")
        XCTAssertTrue(rows[1].label.contains("Companion of work"), "the nested companion says whose it is")
        Self.keepScreenshot(of: app, named: "companion nested under its attached parent")

        rows[0].tap()
        let first = Self.screen(showing: "P1 ready", in: app)
        FakeApp.expect(first, "work's first pane")
        // a tap gives the terminal the keyboard, as a user's does before a chord
        first.tap()
        let down = XCUIKeyboardKey.downArrow.rawValue
        Self.moveSession(down, to: "R1 ready", in: app)
        Self.moveSession(down, to: "N1 ready", in: app)
        Self.moveSession(down, to: "P1 ready", in: app)
        Self.moveSession(XCUIKeyboardKey.upArrow.rawValue, to: "N1 ready", in: app)
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
