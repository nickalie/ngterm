import XCTest

/// The app on the fake core, as the UI tests launch and drive it.
internal enum FakeApp {
    // how long a screen may take to come
    private static let wait: TimeInterval = 30
    // how long a list is watched for, at most, before an audit: this many reads, this far apart
    private static let settleReads = 20
    private static let settleBeat: TimeInterval = 0.25
    // how many reads running a list has to keep still for, its text inside its rows
    private static let settleQuietReads = 3
    // how often a list whose text lies outside its rows is nudged, and how far
    private static let settleNudges = 3
    private static let nudgeDistance: CGFloat = 12
    private static let nudgePress: TimeInterval = 0.05
    // where in a list a nudge starts: its middle
    private static let middle: CGFloat = 0.5
    // the audit's checks that make the app's text larger, and so lay its screen out again while they run
    private static let resizingAudits: XCUIAccessibilityAuditType = [.dynamicType, .textClipped]

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

    /// Tells the app's fake worlds the test has seen what the cue `name` (the app's `FakeCues.Cue`) waits on.
    internal static func cue(_ name: String) {
        let center = CFNotificationCenterGetDarwinNotifyCenter()
        CFNotificationCenterPostNotification(
            center,
            CFNotificationName("com.nickalie.ngterm.fake-cue.\(name)" as CFString),
            nil,
            nil,
            true
        )
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
    ///
    /// On an iPad's form sheet, once the keyboard has gone, the rows' accessibility frames can stay where the keyboard
    /// left them while their text has moved on (seen 20-36 pt apart, for good): the audit then fails text it measures
    /// in a row it isn't in. A list whose text lies outside its rows is nudged, which lays the rows out anew.
    @MainActor
    internal static func settle(_ list: XCUIElement, in app: XCUIApplication) {
        let keyboardGone = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "count == 0"),
            object: app.keyboards
        )
        XCTAssertEqual(XCTWaiter.wait(for: [keyboardGone], timeout: wait), .completed, "the keyboard stayed")
        // past the last nudge the audit has its say
        for nudges in 0...settleNudges {
            if nudges > 0 {
                nudge(list)
            }
            if settleRows(list) {
                return
            }
        }
    }

    // reads the rows' frames and the text's a beat apart until they agree for `settleQuietReads` reads running with
    // every text inside a row; false when they don't come to that
    @MainActor
    private static func settleRows(_ list: XCUIElement) -> Bool {
        var before: [CGRect] = []
        var quiet = 0
        for _ in 0..<settleReads {
            _ = XCTWaiter.wait(for: [XCTestExpectation(description: "a beat")], timeout: settleBeat)
            let rows = list.cells.allElementsBoundByIndex.map(\.frame)
            let texts = list.staticTexts.allElementsBoundByIndex.map(\.frame)
            let now = rows + texts
            let isInRows = texts.allSatisfy { text in rows.contains { row in row.contains(text) } }
            quiet = now == before && isInRows ? quiet + 1 : 0
            if quiet >= settleQuietReads {
                return true
            }
            before = now
        }
        return false
    }

    // drags `list` a few points down and lets go: it bounces back (or scrolls by as much), laying its rows out anew
    @MainActor
    private static func nudge(_ list: XCUIElement) {
        XCTContext.runActivity(named: "nudge the list: its text lies outside its rows") { _ in
            let start = list.coordinate(withNormalizedOffset: CGVector(dx: middle, dy: middle))
            start.press(forDuration: nudgePress, thenDragTo: start.withOffset(CGVector(dx: .zero, dy: nudgeDistance)))
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
        try performAudit(of: app, for: .all) { issue in
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
        try performAudit(of: app, for: .all) { issue in
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
        try performAudit(of: app, for: .all) { issue in
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
        try performAudit(of: app, for: XCUIAccessibilityAuditType.all.subtracting(.dynamicType)) { issue in
            isSetAside(issue, in: app, isSheet: true)
        }
    }

    /// Audits the form `page` as ``auditForm(_:)`` does, setting aside too the contrast of text the page cuts at its
    /// edge, as ``auditSheet(_:page:)`` does: on an iPad's sheet a form's row can run past the sheet's foot.
    @MainActor
    internal static func auditForm(_ app: XCUIApplication, page: XCUIElement) throws {
        let shown = page.frame
        try performAudit(of: app, for: XCUIAccessibilityAuditType.all.subtracting(.dynamicType)) { issue in
            if issue.auditType == .contrast, let element = issue.element, !shown.contains(element.frame) {
                return true
            }
            return isSetAside(issue, in: app, isSheet: true)
        }
    }

    /// Audits `app` for `types` in two passes, `handler` judging each issue: first the checks that look at the screen
    /// as it is, then those that make its text larger to look at it (Dynamic Type, clipped text).
    ///
    /// In one pass, the larger text's checks lay the screen out again while the others look (a screen recording of the
    /// audit shows the form at a large text size mid-audit), and the contrast check can then measure a row's text where
    /// its frame no longer is: an iPad's machine form failed so, its Key rows' contrast, in every run for a while, and
    /// passed in others, as the checks happened to follow each other. Apart, the contrast check only sees the screen at
    /// the user's text size.
    @MainActor
    private static func performAudit(
        of app: XCUIApplication,
        for types: XCUIAccessibilityAuditType,
        _ handler: @escaping (XCUIAccessibilityAuditIssue) throws -> Bool
    ) throws {
        try app.performAccessibilityAudit(for: types.subtracting(resizingAudits), handler)
        let resizing = types.intersection(resizingAudits)
        if !resizing.isEmpty {
            try app.performAccessibilityAudit(for: resizing, handler)
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

    // an iPad's contrast issue in the machines' column, the sidebar of Liquid Glass: once the column beside it (a
    // machine's sessions, or a session) has the focus, the audit measures the rows' colored text against the glass's
    // grey (or the selection it greys), not the grouped ground the colors are made for, whose contrast ContrastTests
    // checks. Whether the focus has moved yet when the audit looks varies from run to run, so it is set aside whenever
    // such a column shows. Only the rows, and only beside a machine's sessions or a session: the machine form and the
    // machines' list on their own are audited in full
    @MainActor
    private static func isSidebarGlass(_ issue: XCUIAccessibilityAuditIssue, in app: XCUIApplication) -> Bool {
        let isPadContrast = UIDevice.current.userInterfaceIdiom == .pad && issue.auditType == .contrast
        guard isPadContrast, let element = issue.element else {
            return false
        }
        // a session's title, or a machine's sessions: their list, or the New session button of a connected machine
        let besideColumn = ["session-title", "sessions", "new-session"].contains { identifier in
            app.descendants(matching: .any)[identifier].firstMatch.exists
        }
        guard besideColumn else {
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
