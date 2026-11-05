import XCTest

/// The app on the fake core, as the UI tests launch and drive it.
internal enum FakeApp {
    // how long a screen may take to come
    private static let wait: TimeInterval = 30
    // how long a list is watched for, at most, before an audit: this many reads, this far apart
    private static let settleReads = 20
    private static let settleBeat: TimeInterval = 0.25

    /// Launches the app on the fake core for `test`: on the first run, or past it with a key when `withKey`.
    ///
    /// An iPad turns on its side for the test, and upright again after it: the three columns then sit side by side,
    /// where upright the machines' column comes over the others as an overlay, and the audit would measure the dimmed
    /// columns under it and the overlay while it slides in. It turns again once the app runs: on a simulator just
    /// booted the app's first window stays upright on the turned device, and an audit then measures a sheet laid out
    /// upright, its last rows off the screen.
    @MainActor
    internal static func launch(withKey: Bool, for test: XCTestCase) -> XCUIApplication {
        launch(withKey: withKey, arguments: [], for: test)
    }

    /// Launches the app as ``launch(withKey:for:)`` does, with the launch arguments `arguments` besides.
    @MainActor
    internal static func launch(withKey: Bool, arguments: [String], for test: XCTestCase) -> XCUIApplication {
        let isPad = UIDevice.current.userInterfaceIdiom == .pad
        if isPad {
            XCUIDevice.shared.orientation = .landscapeLeft
            test.addTeardownBlock { @MainActor in
                XCUIDevice.shared.orientation = .portrait
            }
        }
        let app = XCUIApplication()
        app.launchArguments = ["-ngterm-fake-core"] + (withKey ? ["-ngterm-fake-key"] : []) + arguments
        app.launch()
        if isPad {
            expectLandscape(app)
        }
        return app
    }

    // turns the running app on its side, from upright, and waits until its window is wider than it is tall: a
    // simulator just booted has the device on its side before the app's first window follows it
    @MainActor
    private static func expectLandscape(_ app: XCUIApplication) {
        XCUIDevice.shared.orientation = .portrait
        XCUIDevice.shared.orientation = .landscapeLeft
        let window = app.windows.firstMatch
        for _ in 0..<Int(wait / settleBeat) {
            let frame = window.frame
            if frame.width > frame.height {
                return
            }
            _ = XCTWaiter.wait(for: [XCTestExpectation(description: "a beat")], timeout: settleBeat)
        }
        XCTFail("the app's window stayed upright: \(window.frame)")
    }

    /// Waits for `element`, failing the test when it doesn't come.
    @MainActor
    internal static func expect(_ element: XCUIElement, _ what: String) {
        XCTAssertTrue(element.waitForExistence(timeout: wait), "no \(what)")
    }

    /// The first element whose label holds `text`, whatever its kind.
    @MainActor
    internal static func element(containing text: String, in app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    /// Shows the machines' column: an iPad held upright starts with it hidden, behind Show machines.
    @MainActor
    internal static func showMachines(in app: XCUIApplication) {
        let add = app.buttons["add-machine"]
        let show = app.buttons["Show machines"]
        let either = app.buttons
            .matching(NSPredicate(format: "identifier == 'add-machine' OR label == 'Show machines'"))
            .firstMatch
        expect(either, "machines column")
        if !add.exists, show.exists {
            show.tap()
        }
        expect(add, "Add machine")
    }

    /// Adds a machine at `host` signing in as `user`, through the form, auditing the form first when `audit`.
    @MainActor
    internal static func addMachine(host: String, user: String, in app: XCUIApplication, audit: Bool) throws {
        showMachines(in: app)
        app.buttons["add-machine"].tap()
        let hostField = app.textFields["machine-host"]
        expect(hostField, "Host field")
        hostField.tap()
        hostField.typeText(host)
        let userField = app.textFields["machine-user"]
        userField.tap()
        // return closes the keyboard, so the audit sees the whole form
        userField.typeText(user + "\n")
        if audit {
            // a scroll closes the keyboard (the form dismisses it so), so the audit sees the whole form; an iPad's
            // sheet holds the whole form, and a swipe there only bounces it, which the audit catches mid-move
            let form = app.collectionViews["machine-form"]
            if UIDevice.current.userInterfaceIdiom != .pad {
                form.swipeUp()
            }
            settle(form, in: app)
            try auditForm(app)
        }
        app.buttons["machine-save"].tap()
    }

    /// Waits until the keyboard has gone and `list` has stopped moving, before an audit looks at it.
    ///
    /// An iPad's keyboard covers the form's last rows while it goes, and a list bounces back after a swipe: the audit
    /// would measure the rows' text against the keyboard, or while it moves, and find it under 4.5:1 now and then.
    @MainActor
    internal static func settle(_ list: XCUIElement, in app: XCUIApplication) {
        let keyboardGone = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "count == 0"),
            object: app.keyboards
        )
        XCTAssertEqual(XCTWaiter.wait(for: [keyboardGone], timeout: wait), .completed, "the keyboard stayed")
        // the last row's frame, read until two reads a beat apart agree
        let last = list.cells.allElementsBoundByIndex.last
        var before = last?.frame
        for _ in 0..<settleReads {
            _ = XCTWaiter.wait(for: [XCTestExpectation(description: "a beat")], timeout: settleBeat)
            let now = last?.frame
            if now == before {
                return
            }
            before = now
        }
    }

    /// Waits until the row of the machine titled `title` reads `badge` (its link's word).
    @MainActor
    internal static func expectRow(_ title: String, reads badge: String, in app: XCUIApplication) {
        let reading = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier == %@ AND label CONTAINS %@", "machine-row-\(title)", badge))
            .firstMatch
        expect(reading, "\(title) reading \(badge)")
    }

    /// Audits the screen, setting aside only what the system's navigation bar draws.
    ///
    /// Its Liquid Glass buttons keep a fixed size by design (the large content viewer shows them at large text
    /// sizes), and the audit measures their text against the glass rather than the tint, whose contrast
    /// ContrastTests checks against every ground.
    @MainActor
    internal static func audit(_ app: XCUIApplication) throws {
        try app.performAccessibilityAudit { issue in
            isSetAside(issue, in: app, isSheet: false)
        }
    }

    /// Audits a sheet as ``audit(_:)`` does, setting aside too the text the audit sees behind it with no element.
    ///
    /// On an iPad a sheet floats over the dimmed app, which VoiceOver rightly doesn't reach while the sheet is up; the
    /// audit, finding text on screen it can't reach, reports it with no element, and so does everything else it finds
    /// of that text (its contrast against the dimming, its lines cut by the sheet).
    @MainActor
    internal static func auditSheet(_ app: XCUIApplication) throws {
        try app.performAccessibilityAudit { issue in
            isSetAside(issue, in: app, isSheet: true)
        }
    }

    /// Audits a sheet whose page is the scroll view `page`, as ``auditSheet(_:)`` does, setting aside too the contrast
    /// of text the page cuts at its edge.
    ///
    /// On an iPad a sheet's page runs on past the sheet's foot, and the audit measures the text cut there against what
    /// lies past the edge (the dimmed app), not the page's ground.
    @MainActor
    internal static func auditSheet(_ app: XCUIApplication, page: XCUIElement) throws {
        let shown = page.frame
        try app.performAccessibilityAudit { issue in
            if issue.auditType == .contrast, let element = issue.element, !shown.contains(element.frame) {
                return true
            }
            return isSetAside(issue, in: app, isSheet: true)
        }
    }

    /// Audits the machine form for everything but Dynamic Type, setting aside what ``audit(_:)`` does.
    ///
    /// The form is a list, which lets go of the rows the audit's larger text pushes off the screen: it then reports
    /// them as not scaling, and measures their contrast while the list lays them out again. Every font of the form is a
    /// Dynamic Type style, and its golden at an accessibility text size (MachinesSnapshotTests) shows them scaling.
    @MainActor
    internal static func auditForm(_ app: XCUIApplication) throws {
        try app.performAccessibilityAudit(for: XCUIAccessibilityAuditType.all.subtracting(.dynamicType)) { issue in
            isSetAside(issue, in: app, isSheet: true)
        }
    }

    // whether the audit sets `issue` aside (the navigation bar's glass, and over a sheet on an iPad the app behind it);
    // one it keeps is named in the report, so a failure says which element it is
    @MainActor
    private static func isSetAside(
        _ issue: XCUIAccessibilityAuditIssue,
        in app: XCUIApplication,
        isSheet: Bool
    ) -> Bool {
        if isNavigationBarGlass(issue, in: app) || isSidebarGlass(issue, in: app) {
            return true
        }
        // what lies behind a sheet has no element the audit can name: text it can't reach, and the dimmed app's text
        // it measures against the dimming or finds cut by the sheet; everything in the sheet has its element
        let isBehindSheet = isSheet && issue.element == nil
        if isBehindSheet, UIDevice.current.userInterfaceIdiom == .pad {
            return true
        }
        let element = issue.element.map { "\($0.elementType.rawValue) '\($0.label)' \($0.frame)" } ?? "none"
        // the activity's name is the record, with the screen as the audit saw it
        XCTContext.runActivity(named: "AUDIT \(issue.compactDescription): \(element)") { activity in
            let shot = XCTAttachment(screenshot: app.screenshot())
            shot.lifetime = .keepAlways
            activity.add(shot)
        }
        return false
    }

    // an iPad's contrast issue in the machines' column, the sidebar of Liquid Glass: once the session beside it has the
    // focus, the audit measures the rows' colored text against the glass's grey (or the selection it greys), not the
    // grouped ground the colors are made for, whose contrast ContrastTests checks. Only the rows, and only beside a
    // session: the machine form and the machines' list on their own are audited in full
    @MainActor
    private static func isSidebarGlass(_ issue: XCUIAccessibilityAuditIssue, in app: XCUIApplication) -> Bool {
        let isPadContrast = UIDevice.current.userInterfaceIdiom == .pad && issue.auditType == .contrast
        guard isPadContrast, let element = issue.element else {
            return false
        }
        guard app.descendants(matching: .any)["session-title"].firstMatch.exists else {
            return false
        }
        let isRow = NSPredicate(
            format: "identifier BEGINSWITH 'machine-row-' OR identifier BEGINSWITH 'sidebar-session-'"
        )
        let rows = app.descendants(matching: .any).matching(isRow)
        return rows.allElementsBoundByIndex.contains { row in row.frame.contains(element.frame) }
    }

    // an issue of the navigation bar's Liquid Glass: its buttons' size and contrast against the glass, its title
    // clipped in a narrow column
    @MainActor
    private static func isNavigationBarGlass(_ issue: XCUIAccessibilityAuditIssue, in app: XCUIApplication) -> Bool {
        guard let element = issue.element, [.dynamicType, .contrast, .textClipped].contains(issue.auditType) else {
            return false
        }
        return app.navigationBars.allElementsBoundByIndex.contains { bar in bar.frame.contains(element.frame) }
    }
}
