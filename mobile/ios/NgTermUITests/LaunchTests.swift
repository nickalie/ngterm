import XCTest

/// The launch smoke test: the app starts on the fake core and shows its first screen.
internal final class LaunchTests: XCTestCase {
    @MainActor
    internal func testLaunchesOnTheFakeCore() {
        let app = XCUIApplication()
        app.launchArguments = ["-ngterm-fake-core"]
        app.launch()
        let version = app.staticTexts["core-version"]
        XCTAssertTrue(version.waitForExistence(timeout: 30))
        XCTAssertEqual(version.label, "core 0.0.0-fake")
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
