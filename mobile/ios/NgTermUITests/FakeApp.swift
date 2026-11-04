import XCTest

/// The app on the fake core, as the UI tests launch and drive it.
internal enum FakeApp {
    // how long a screen may take to come
    private static let wait: TimeInterval = 30

    /// Launches the app on the fake core for `test`: on the first run, or past it with a key when `withKey`.
    ///
    /// An iPad turns on its side for the test, and upright again after it: the three columns then sit side by side,
    /// where upright the machines' column comes over the others as an overlay, and the audit would measure the dimmed
    /// columns under it and the overlay while it slides in.
    @MainActor
    internal static func launch(withKey: Bool, for test: XCTestCase) -> XCUIApplication {
        if UIDevice.current.userInterfaceIdiom == .pad {
            XCUIDevice.shared.orientation = .landscapeLeft
            test.addTeardownBlock { @MainActor in
                XCUIDevice.shared.orientation = .portrait
            }
        }
        let app = XCUIApplication()
        app.launchArguments = ["-ngterm-fake-core"] + (withKey ? ["-ngterm-fake-key"] : [])
        app.launch()
        return app
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
            // a scroll closes the keyboard (the form dismisses it so), so the audit sees the whole form
            app.collectionViews["machine-form"].swipeUp()
            // the scroll settles before the audit looks
            _ = XCTWaiter.wait(for: [XCTestExpectation(description: "settled")], timeout: 1)
            try auditForm(app)
        }
        app.buttons["machine-save"].tap()
    }

    /// Waits until the row of the machine titled `title` reads `badge` (its link's word).
    @MainActor
    internal static func expectRow(_ title: String, reads badge: String, in app: XCUIApplication) {
        let reading = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier == %@ AND label CONTAINS %@", "machine-\(title)", badge))
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
    /// audit, finding text on screen it can't reach, reports it with no element.
    @MainActor
    internal static func auditSheet(_ app: XCUIApplication) throws {
        try app.performAccessibilityAudit { issue in
            isSetAside(issue, in: app, isSheet: true)
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
        if isNavigationBarGlass(issue, in: app) {
            return true
        }
        let isBehindSheet = isSheet && issue.element == nil && issue.auditType == .elementDetection
        if isBehindSheet, UIDevice.current.userInterfaceIdiom == .pad {
            return true
        }
        let element = issue.element.map { "\($0.elementType.rawValue) '\($0.label)' \($0.frame)" } ?? "none"
        XCTContext.runActivity(named: "AUDIT \(issue.compactDescription): \(element)") { _ in
            // the activity's name is the record
        }
        return false
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
