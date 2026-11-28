import XCTest

/// Settings' connection timeout and a machine's own, through the app's real core, against an ssh server that never
/// answers (mobile/android's `ConnectTimeoutE2eTest`).
///
/// From a fresh install a person imports their key, sets Settings > Connections > Connection timeout to 3 s, its
/// least, and adds two machines on ports of the Mac's 127.0.0.1 that take the connection and never say a word
/// (``SilentPort``): one with no timeout of its own, one with 7 s of its own. The core gives up on each after its
/// timeout: the port sees the app close the connection 3 s and 7 s after it took it. After a relaunch, Settings still
/// reads 3 s, the forms still hold 7 s and nothing, and the connects at launch give up after the same times.
///
/// `scripts/test-ios-e2e.sh` installs the app anew and hands the key to the test through the test runner's
/// environment. Without it the test is skipped.
internal final class ConnectTimeoutOnAMachineTests: XCTestCase {
    private static let settingsLabel = "qa-to-settings"
    private static let ownLabel = "qa-to-own"
    // the slider's left end (AppSettings.connectTimeouts)
    private static let settingsSeconds = 3
    private static let ownSeconds = 7
    // how far a give-up may come before its timeout, and after it
    private static let early: TimeInterval = 0.3
    private static let late: TimeInterval = 2.5
    // how long the test waits for a connect to give up
    private static let giveUpWait: TimeInterval = 40
    private static let pageWait: TimeInterval = 5
    private static let tries = 3
    private static let swipes = 12
    private static let longPress: TimeInterval = 1
    // past this a test fails: on a simulator that answers a minute late, waits counted in reads take hours
    private static let timeLimit: TimeInterval = 1_200

    // the connections a port took and saw closed, whole: not those the app's end cut
    private static func closed(_ calls: [SilentPort.Call], from start: TimeInterval, to end: TimeInterval) -> [Double] {
        calls.filter { $0.taken >= start && ($0.closed ?? .infinity) <= end }.compactMap(\.seconds)
    }

    private static func rounded(_ seconds: [Double]) -> String {
        seconds.map { String(format: "%.2f s", $0) }.joined(separator: ", ")
    }

    @MainActor
    internal func testTheConnectGivesUpAfterSettingsOrItsOwnTimeoutAndBothStayAfterARelaunch() throws {
        executionTimeAllowance = Self.timeLimit
        continueAfterFailure = false
        let machine = try E2EMachine.fromEnvironment()
        let toSettings = try SilentPort()
        let toOwn = try SilentPort()
        addTeardownBlock {
            toSettings.stop()
            toOwn.stop()
        }
        var app = FakeApp.launchOnTheRealCore(for: self)
        let start = app.buttons["import-key"]
        FakeApp.expect(start, "the welcome's Import a key: is the install fresh?")
        start.tap()
        importKey(machine, in: app)

        openSettings(in: app)
        setTimeoutToTheFewest(in: app)
        shot("1-settings-3s", app)
        app.buttons["settings-done"].tap()

        let first = ProcessInfo.processInfo.systemUptime
        addMachine(Self.settingsLabel, at: machine.host, port: toSettings.port, user: machine.user, own: nil, in: app)
        addMachine(Self.ownLabel, at: machine.host, port: toOwn.port, user: machine.user, own: Self.ownSeconds, in: app)
        expectGiveUp(on: toSettings, after: Self.settingsSeconds, since: first, "Settings' timeout")
        expectGiveUp(on: toOwn, after: Self.ownSeconds, since: first, "the machine's own timeout")
        shot("2-given-up", app)

        let end = ProcessInfo.processInfo.systemUptime
        app.terminate()
        let relaunch = ProcessInfo.processInfo.systemUptime
        app = FakeApp.launchOnTheRealCore(for: self)
        FakeApp.showMachines(in: app)
        expectGiveUp(on: toSettings, after: Self.settingsSeconds, since: relaunch, "Settings' timeout, relaunched")
        expectGiveUp(on: toOwn, after: Self.ownSeconds, since: relaunch, "its own timeout, relaunched")
        assertEveryGiveUp(of: toSettings, took: Self.settingsSeconds, first: first, end: end, relaunch: relaunch)
        assertEveryGiveUp(of: toOwn, took: Self.ownSeconds, first: first, end: end, relaunch: relaunch)

        openSettings(in: app)
        let slider = app.sliders["connect-timeout"]
        FormScroll.scroll(to: slider, in: app.collectionViews["settings-form"], swipes: Self.swipes)
        XCTAssertEqual(slider.value as? String, "\(Self.settingsSeconds) seconds", "Settings' timeout, relaunched")
        shot("3-settings-relaunched", app)
        app.buttons["settings-done"].tap()

        let own = openEdit(Self.ownLabel, in: app)
        XCTAssertEqual(own.value as? String, "\(Self.ownSeconds)", "the machine's own timeout, relaunched")
        shot("4-own-relaunched", app)
        app.buttons["Cancel"].tap()
        let none = openEdit(Self.settingsLabel, in: app)
        XCTAssertEqual(none.placeholderValue, "\(Self.settingsSeconds) s from Settings")
        XCTAssertEqual(none.value as? String, none.placeholderValue, "a machine with no timeout of its own, relaunched")
        shot("5-none-relaunched", app)
        app.buttons["Cancel"].tap()
    }

    // on the import screen: the machine's key and its passphrase, Import, and the public key page closed
    @MainActor
    private func importKey(_ machine: E2EMachine, in app: XCUIApplication) {
        let field = app.textViews["private-key"]
        FakeApp.expect(field, "the private key's field")
        field.tap()
        field.typeText(machine.key.trimmingCharacters(in: .whitespacesAndNewlines))
        let secret = app.secureTextFields["passphrase"]
        SecureEntry.type(machine.passphrase, into: secret)
        secret.typeText("\n")
        app.buttons["import-key-action"].tap()
        let page = app.descendants(matching: .any)["public-key-page"].firstMatch
        FakeApp.expect(page, "the imported key's public half")
        let done = app.buttons["key-done"]
        FakeApp.expect(done, "the public key page's Done")
        for _ in 0..<Self.tries where ScreenRead.now({ ScreenRead.isGone(page) }) == .doesNotHold {
            done.tap()
            ScreenRead.step(lasting: Self.pageWait).poll { ScreenRead.isGone(page) }
        }
        FakeApp.expectGone(page, "the public key page, after Done,")
    }

    @MainActor
    private func openSettings(in app: XCUIApplication) {
        FakeApp.showMachines(in: app)
        app.buttons["settings"].tap()
        FakeApp.expect(app.navigationBars["Settings"], "Settings")
    }

    // the slider at its left end: an XCTest move lands only near a position inside the slider, but sure at its ends
    @MainActor
    private func setTimeoutToTheFewest(in app: XCUIApplication) {
        let slider = app.sliders["connect-timeout"]
        FormScroll.scroll(to: slider, in: app.collectionViews["settings-form"], swipes: Self.swipes)
        slider.adjust(toNormalizedSliderPosition: 0)
        XCTAssertEqual(slider.value as? String, "\(Self.settingsSeconds) seconds")
    }

    // the machine through the form, with `own` seconds of its own (nil: none); it connects as it is saved
    @MainActor
    private func addMachine(
        _ label: String,
        at host: String,
        port: UInt16,
        user: String,
        own: Int?,
        in app: XCUIApplication
    ) {
        FakeApp.showMachines(in: app)
        app.buttons["add-machine"].tap()
        E2EMachine.fill("machine-label", with: label, in: app)
        E2EMachine.fill("machine-host", with: host, in: app)
        E2EMachine.fill("machine-port", with: String(port), in: app)
        E2EMachine.fill("machine-user", with: user, in: app)
        if let own {
            // Return closes the keyboard, so the form scrolls to the field with nothing over it
            app.textFields["machine-user"].typeText("\n")
            let field = app.textFields["machine-timeout"]
            FormScroll.scroll(to: field, in: app.collectionViews["machine-form"], swipes: Self.swipes)
            E2EMachine.fill("machine-timeout", with: String(own), in: app)
        }
        app.buttons["machine-save"].tap()
        FakeApp.expectGone(app.collectionViews["machine-form"], "the form of \(label), after Save,")
    }

    // the form of the machine `label`, from its row's menu; its timeout field
    @MainActor
    private func openEdit(_ label: String, in app: XCUIApplication) -> XCUIElement {
        FakeApp.showMachines(in: app)
        let row = app.descendants(matching: .any)["machine-row-\(label)"].firstMatch
        FakeApp.expect(row, "the row of \(label)")
        row.press(forDuration: Self.longPress)
        let edit = app.buttons["Edit"]
        FakeApp.expect(edit, "Edit in the menu of \(label)")
        edit.tap()
        let field = app.textFields["machine-timeout"]
        FakeApp.expect(field, "the timeout field of \(label)")
        FormScroll.scroll(to: field, in: app.collectionViews["machine-form"], swipes: Self.swipes)
        return field
    }

    // waits for a connection `port` took after `since` to close, and asserts it closed `seconds` after it was taken
    @MainActor
    private func expectGiveUp(on port: SilentPort, after seconds: Int, since: TimeInterval, _ what: String) {
        let deadline = Date(timeIntervalSinceNow: Self.giveUpWait)
        var gaveUp: TimeInterval?
        while gaveUp == nil, Date() < deadline {
            gaveUp = port.calls.first { $0.taken >= since }?.seconds
            if gaveUp == nil {
                _ = XCTWaiter.wait(for: [XCTestExpectation(description: "a beat")], timeout: ScreenRead.beat)
            }
        }
        guard let gaveUp else {
            XCTFail("\(what): no connect gave up in \(Int(Self.giveUpWait)) s: \(port.calls)")
            return
        }
        XCTContext.runActivity(named: "\(what): gave up after \(String(format: "%.2f", gaveUp)) s") { _ in
            assertNear(gaveUp, seconds, what)
        }
    }

    // every connection the port saw close, before the app's end and after its relaunch, gave up after `seconds`
    private func assertEveryGiveUp(
        of port: SilentPort,
        took seconds: Int,
        first: TimeInterval,
        end: TimeInterval,
        relaunch: TimeInterval
    ) {
        let before = Self.closed(port.calls, from: first, to: end)
        let after = Self.closed(port.calls, from: relaunch, to: .infinity)
        XCTAssertFalse(before.isEmpty, "no connect gave up before the relaunch")
        XCTAssertFalse(after.isEmpty, "no connect gave up after the relaunch")
        for taken in before + after {
            assertNear(taken, seconds, "a give-up of \(port.calls)")
        }
        let told = XCTAttachment(
            string: "\(seconds) s: before the relaunch \(Self.rounded(before)); after it \(Self.rounded(after))"
        )
        told.name = "give-ups-\(seconds)s"
        told.lifetime = .keepAlways
        add(told)
    }

    private func assertNear(_ taken: TimeInterval, _ seconds: Int, _ what: String) {
        let low = TimeInterval(seconds) - Self.early
        let high = TimeInterval(seconds) + Self.late
        XCTAssertTrue((low...high).contains(taken), "\(what): gave up after \(taken) s, not \(seconds) s")
    }

    @MainActor
    private func shot(_ name: String, _ app: XCUIApplication) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched, and the teardown stops the ports
    }
}
