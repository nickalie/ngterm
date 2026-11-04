import XCTest

/// The launch smoke test and the frame's accessibility audit, on the fake core.
internal final class LaunchTests: XCTestCase {
    @MainActor
    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-ngterm-fake-core"]
        app.launch()
        let firstScreen = app.buttons
            .matching(NSPredicate(format: "identifier == 'settings' OR label == 'Show machines'"))
            .firstMatch
        XCTAssertTrue(firstScreen.waitForExistence(timeout: 30))
        return app
    }

    // an iPad held upright starts on the sessions column, its machines hidden: Show machines brings them in
    @MainActor
    private func openSettings(in app: XCUIApplication) {
        let settings = app.buttons["settings"]
        if !settings.exists {
            app.buttons["Show machines"].tap()
            XCTAssertTrue(settings.waitForExistence(timeout: 10))
        }
        settings.tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
    }

    @MainActor
    internal func testLaunchesOnTheFakeCore() {
        let app = launch()
        openSettings(in: app)
        let version = app.descendants(matching: .any)["core-version"]
        XCTAssertTrue(version.waitForExistence(timeout: 10))
        XCTAssertTrue(version.label.contains("0.0.0-fake"), version.label)
    }

    // Audits every screen, setting aside only what the system's navigation bar draws: its Liquid Glass buttons keep a
    // fixed size by design (the large content viewer shows them at large text sizes), and the audit measures their
    // text against the glass rather than the tint, whose contrast ContrastTests checks against every ground.
    @MainActor
    private func audit(_ app: XCUIApplication) throws {
        try app.performAccessibilityAudit { issue in
            guard let element = issue.element, [.dynamicType, .contrast].contains(issue.auditType) else {
                return false
            }
            return app.navigationBars.allElementsBoundByIndex.contains { bar in bar.frame.contains(element.frame) }
        }
    }

    @MainActor
    internal func testTheFramePassesTheAccessibilityAudit() throws {
        let app = launch()
        try audit(app)
        openSettings(in: app)
        try audit(app)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
