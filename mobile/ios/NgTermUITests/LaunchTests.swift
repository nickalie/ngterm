import XCTest

/// The launch smoke test and the frame's accessibility audit, on the fake core.
internal final class LaunchTests: XCTestCase {
    @MainActor
    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        // with a key, so the first run is over (FirstRunTests walk through it)
        app.launchArguments = ["-ngterm-fake-core", "-ngterm-fake-key"]
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

    @MainActor
    internal func testTheFramePassesTheAccessibilityAudit() throws {
        let app = launch()
        try FakeApp.audit(app)
        openSettings(in: app)
        try FakeApp.audit(app)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
