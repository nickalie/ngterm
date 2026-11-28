import XCTest

/// Settings on the fake core, with the machine `split` signing in with the app's key: a setting changed sticks, the
/// connection timeout sticks and a new machine's form names it, a machine's own timeout outside 3 to 60 s is refused
/// and one inside it is kept until it is cleared, every notification switch turns off and stays off, a rotated key's
/// public half offers to switch the machine to it, and Settings and that page are audited.
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

    @MainActor
    private func scroll(to element: XCUIElement, in app: XCUIApplication) {
        FormScroll.scroll(to: element, in: app.collectionViews["settings-form"], swipes: Self.swipes)
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

    /// Touches the switch inside `row` until the row reads `value`, leaving the caller to assert that it does.
    ///
    /// The touch lands on the switch itself, at the row's trailing end. UISwitch starts its press on a 10 ms timer, and
    /// a tap lasts 67 ms: when XCTest's own accessibility snapshots hold the app's main thread for longer, as on a
    /// loaded host, the touch's down and up are handled together, the timer never runs and the switch ignores the
    /// touch. A 0.6 s press outlasts most of those stalls; one that doesn't is repeated. A dead switch still fails the
    /// assert.
    @MainActor
    private func flip(_ row: XCUIElement, to value: String) {
        let flipped = NSPredicate(format: "value == %@", value)
        for _ in 0..<3 {
            row.switches.firstMatch.press(forDuration: 0.6)
            let read = ScreenRead.step(lasting: 3).poll { ScreenRead.shows(row, matching: flipped) }
            if read.held {
                return
            }
        }
    }

    @MainActor
    internal func testAChangedSettingSticks() throws {
        let app = openSettings()
        let keepOn = app.switches["keep-screen-on"]
        FakeApp.expect(keepOn, "Keep the screen on")
        XCTAssertEqual(keepOn.value as? String, "0")
        flip(keepOn, to: "1")
        XCTAssertEqual(keepOn.value as? String, "1")
        try FakeApp.auditForm(app, page: app.collectionViews["settings-form"])

        app.buttons["settings-done"].tap()
        app.buttons["settings"].tap()
        FakeApp.expect(keepOn, "Keep the screen on, again")
        XCTAssertEqual(keepOn.value as? String, "1")
    }

    @MainActor
    internal func testTheConnectionTimeoutSticksAndIsWhatANewMachineTakes() {
        let app = openSettings()
        let timeout = app.sliders["connect-timeout"]
        scroll(to: timeout, in: app)
        XCTAssertEqual(timeout.value as? String, "10 seconds")
        // the left end: on an iPad a move to the right end landed on 59 s now and then
        timeout.adjust(toNormalizedSliderPosition: 0)
        XCTAssertEqual(timeout.value as? String, "3 seconds")

        app.buttons["settings-done"].tap()
        app.buttons["settings"].tap()
        scroll(to: timeout, in: app)
        XCTAssertEqual(timeout.value as? String, "3 seconds", "the timeout, again")
        app.buttons["settings-done"].tap()

        app.buttons["add-machine"].tap()
        let own = app.textFields["machine-timeout"]
        FormScroll.scroll(to: own, in: app.collectionViews["machine-form"], swipes: Self.swipes)
        XCTAssertEqual(own.placeholderValue, "3 s from Settings")
        XCTAssertEqual(own.label, "Connection timeout", "VoiceOver names the field, not the hint")
    }

    @MainActor
    internal func testAMachinesOwnTimeoutIsCheckedKeptAndCleared() throws {
        let app = FakeApp.launch(withKey: true, for: self)
        FakeApp.showMachines(in: app)
        app.buttons["add-machine"].tap()
        E2EMachine.fill("machine-host", with: "build-01.lan", in: app)
        E2EMachine.fill("machine-user", with: "nick", in: app)
        // Return closes the keyboard, so the form scrolls to the timeout with nothing over it
        app.textFields["machine-user"].typeText("\n")
        let form = app.collectionViews["machine-form"]
        let own = app.textFields["machine-timeout"]
        let problem = FakeApp.element(containing: "Enter 3 to 60 seconds, or leave it empty.", in: app)
        // scrolled to once: the number pad has no Return, and a scroll under it would close it
        FormScroll.scroll(to: own, in: form, swipes: Self.swipes)
        for wrong in ["2", "61"] {
            E2EMachine.fill("machine-timeout", with: wrong, in: app)
            app.buttons["machine-save"].tap()
            FakeApp.expect(problem, "the problem under a timeout of \(wrong)")
            XCTAssertTrue(form.exists, "Save took a timeout of \(wrong)")
        }
        // the number pad closes, so the audit measures no row against it: a phone's with a drag of the form; an iPad's
        // stays for a tap outside it, so the user field takes the focus and its Return lets it go
        if UIDevice.current.userInterfaceIdiom == .pad {
            let user = app.textFields["machine-user"]
            user.tap()
            user.typeText("\n")
        } else {
            form.swipeDown()
        }
        FakeApp.settle(form, in: app)
        FakeApp.attachScreen("the problem under a timeout of 61")
        try FakeApp.auditForm(app, page: form)

        FormScroll.scroll(to: own, in: form, swipes: Self.swipes)
        E2EMachine.fill("machine-timeout", with: "45", in: app)
        FakeApp.expectGone(problem, "the problem under a timeout of 45")
        app.buttons["machine-save"].tap()
        let trust = app.buttons["trust-host-key"]
        FakeApp.expect(trust, "host key sheet")
        trust.tap()
        FakeApp.expectRow("build-01.lan", reads: "Connected", in: app)

        MachineMenu.edit("build-01.lan", in: app)
        XCTAssertEqual(own.value as? String, "45", "the machine's own timeout")
        E2EMachine.fill("machine-timeout", with: String(repeating: XCUIKeyboardKey.delete.rawValue, count: 2), in: app)
        app.buttons["machine-save"].tap()
        FakeApp.expectGone(form, "the form, after Save,")
        FakeApp.expectRow("build-01.lan", reads: "Connected", in: app)

        MachineMenu.edit("build-01.lan", in: app)
        XCTAssertEqual(own.placeholderValue, "10 s from Settings")
        XCTAssertEqual(own.value as? String, own.placeholderValue, "a cleared timeout takes Settings'")
    }

    @MainActor
    internal func testEveryNotificationSwitchTurnsOffAndSticks() {
        let app = openSettings()
        let kinds = ["questions", "waiting", "done", "notices", "errors"]
        for kind in kinds {
            let row = app.switches["notice-\(kind)"]
            scroll(to: row, in: app)
            XCTAssertEqual(row.value as? String, "1", "\(kind) is on by default")
            flip(row, to: "0")
            XCTAssertEqual(row.value as? String, "0", "\(kind) turned off")
        }

        app.buttons["settings-done"].tap()
        app.buttons["settings"].tap()
        for kind in kinds {
            let row = app.switches["notice-\(kind)"]
            scroll(to: row, in: app)
            XCTAssertEqual(row.value as? String, "0", "\(kind) stays off")
        }
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
        // and the new key, the row after it, is split's: on an iPad's sheet that row can still be below the foot, where
        // the list hasn't made it yet
        scroll(to: FakeApp.element(containing: "Used by split", in: app), in: app)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
