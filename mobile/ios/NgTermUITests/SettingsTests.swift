import XCTest

/// Settings on the fake core, with the machine `split` signing in with the app's key: a setting changed sticks, a
/// rotated key's public half offers to switch the machine to it, and Settings and that page are audited.
internal final class SettingsTests: XCTestCase {
    // how many swipes a row far down the form may take to come into view
    private static let swipes = 12
    // how long an element is watched for, at most, before an audit: this many reads, this far apart
    private static let settleReads = 20
    private static let settleBeat: TimeInterval = 0.5

    // the app past its first run, Settings open
    @MainActor
    private func openSettings() -> XCUIApplication {
        let app = FakeApp.launch(withKey: true, arguments: ["-ngterm-fake-session"], for: self)
        FakeApp.showMachines(in: app)
        app.buttons["settings"].tap()
        FakeApp.expect(app.navigationBars["Settings"], "Settings")
        return app
    }

    // swipes the form up until `element` is on screen
    @MainActor
    private func scroll(to element: XCUIElement, in app: XCUIApplication) {
        let form = app.collectionViews["settings-form"]
        for _ in 0..<Self.swipes where !(element.exists && element.isHittable) {
            form.swipeUp()
        }
        XCTAssertTrue(element.isHittable, "\(element) never came into view")
    }

    // waits until `element` has stopped moving (a push, a scroll's bounce), so an audit doesn't measure it mid-move
    @MainActor
    private func settle(_ element: XCUIElement) {
        var before = element.frame
        for _ in 0..<Self.settleReads {
            _ = XCTWaiter.wait(for: [XCTestExpectation(description: "a beat")], timeout: Self.settleBeat)
            let now = element.frame
            if now == before {
                return
            }
            before = now
        }
    }

    @MainActor
    internal func testAChangedSettingSticks() throws {
        let app = openSettings()
        let keepOn = app.switches["keep-screen-on"]
        FakeApp.expect(keepOn, "Keep the screen on")
        XCTAssertEqual(keepOn.value as? String, "0")
        // the switch itself, at the row's trailing end
        keepOn.switches.firstMatch.tap()
        XCTAssertEqual(keepOn.value as? String, "1")
        try FakeApp.auditForm(app)

        app.buttons["settings-done"].tap()
        app.buttons["settings"].tap()
        FakeApp.expect(keepOn, "Keep the screen on, again")
        XCTAssertEqual(keepOn.value as? String, "1")
    }

    @MainActor
    internal func testARotatedKeyIsUsedForEveryMachine() throws {
        let app = openSettings()
        let rotate = app.buttons["rotate-key"]
        scroll(to: rotate, in: app)
        rotate.tap()

        let use = app.buttons["use-key-everywhere"]
        FakeApp.expect(app.navigationBars["Public key"], "the new key's public half")
        let page = app.scrollViews["public-key-page"]
        // audited at its top once the push has landed: scrolled, its text would pass under the navigation bar's glass
        settle(page.staticTexts.firstMatch)
        try FakeApp.auditSheet(app, page: page)
        for _ in 0..<Self.swipes where !(use.exists && use.isHittable) {
            page.swipeUp()
        }
        FakeApp.expect(use, "Use for every machine")
        use.tap()
        FakeApp.expect(app.staticTexts["key-everywhere"], "that every machine signs in with the key")

        // back in Settings, the old key is no machine's
        app.navigationBars.buttons.element(boundBy: 0).tap()
        let unused = FakeApp.element(containing: "No machine uses it", in: app)
        scroll(to: unused, in: app)
        XCTAssertTrue(FakeApp.element(containing: "Used by split", in: app).exists)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
