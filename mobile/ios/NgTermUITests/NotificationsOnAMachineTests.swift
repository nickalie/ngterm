import XCTest

/// The notification switches of Settings on a real machine, through the app's real core and the system's notifications.
///
/// From a fresh install a person imports their key, adds the machine, opens a named session and turns notifications
/// on. A program in the session posts a notice and the person leaves the session: the notice shows as a banner and
/// stays in the system's delivered list. The app starts again, and Notices turned off takes that notice of the earlier
/// run down. An agent's question out of sight shows the same way: Questions turned off takes it down, and turned on
/// again posts it again. With every switch off, a question and a notice out of sight post nothing, though the question
/// is open in the session.
///
/// A banner is read from SpringBoard. What stays up is read from the system's delivered list of the app, in the
/// simulator's files on the Mac.
/// A phone leaves the session for the machines; an iPad shows the session beside them, so it goes to another session,
/// made before the first post: the post's delay must not run while a session is made.
/// The machine is a private `ngterm-headless` behind a private `sshd`, which `scripts/test-ios-e2e.sh` starts. Without
/// a machine the test is skipped.
internal final class NotificationsOnAMachineTests: XCTestCase {
    private static let wait: TimeInterval = 60
    private static let offerWait: TimeInterval = 10
    // how long nothing may show after a post with every switch off
    private static let quiet: TimeInterval = 30
    // how long a program waits before it posts: the test leaves the session meanwhile
    private static let delay = 10
    private static let menuPress: TimeInterval = 1
    private static let label = "qa-ntf-mac"
    private static let session = "qa-ntf-sess"
    private static let other = "qa-ntf-other"
    // what each notification of the test says, in its title or text
    private static let bannerMark = "QA"
    private static let kinds = ["questions", "waiting", "done", "notices", "errors"]
    private static let springboardIdentifier = "com.apple.springboard"

    @MainActor private static var springboard: XCUIApplication {
        XCUIApplication(bundleIdentifier: springboardIdentifier)
    }

    @MainActor private static var isPad: Bool {
        UIDevice.current.userInterfaceIdiom == .pad
    }

    // one ngtermctl call with its options after the subcommand, as ngtermctl takes them
    private static func targeted(_ call: String) -> String {
        "\(call) --target \"$NGTERM_SESSION_ID\" --socket \"$NGTERM_SOCKET\""
    }

    // whether the system's delivered list of the app holds `text`, read from the simulator's own files: XCTest can't
    // read another app's notifications, and Notification Center pulled down shows a banner's text as well
    private static func deliveredHolds(_ text: String) -> Bool {
        let home = URL(fileURLWithPath: NSHomeDirectory())
        // the runner's home is <device>/data/Containers/Data/Application/<id>
        let data = (0..<4).reduce(home) { url, _ in url.deletingLastPathComponent() }
        let lists = data.appendingPathComponent("Library/UserNotifications")
        let stores = (try? FileManager.default.contentsOfDirectory(at: lists, includingPropertiesForKeys: nil)) ?? []
        let needle = Data(text.utf8)
        return stores.contains { store in
            let file = store.appendingPathComponent("DeliveredNotifications.plist")
            return (try? Data(contentsOf: file))?.contains(needle) == true
        }
    }

    // every session's title has the same identifier, so only its label tells the sessions apart
    @MainActor
    private static func title(of name: String, in app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier == %@ AND label CONTAINS %@", "session-title", name))
            .firstMatch
    }

    @MainActor
    internal func testSwitchesTurnedOffTakeTheirNotificationsDownAndPostNothing() throws {
        let machine = try E2EMachine.fromEnvironment()
        var app = FakeApp.launchOnTheRealCore(for: self)
        signIn(with: machine, in: app)
        addMachine(machine, in: app)
        openANewSession(Self.session, allowing: true, in: app)
        makeTheOtherSession(in: app)

        // a notice of the earlier run goes when Notices is turned off
        post("ngtermctl notify qa-ntc-one --title QA", in: app)
        leaveTheSession(in: app)
        expectBanner("qa-ntc-one")
        expectDelivered("qa-ntc-one", isUp: true)
        shot("01-notice-up")
        app.terminate()
        app = FakeApp.launchOnTheRealCore(for: self)
        FakeApp.showMachines(in: app)
        setSwitches(["notices": false], in: app)
        expectDelivered("qa-ntc-one", isUp: false)
        shot("02-notices-off-took-it-down")

        // a question out of sight goes when Questions is turned off, and comes again when it is turned on
        enterTheSession(in: app)
        post("ngtermctl ask open 'QA q-one?' --button yes --button no", in: app)
        leaveTheSession(in: app)
        expectBanner("QA q-one?")
        expectDelivered("QA q-one?", isUp: true)
        shot("03-question-up")
        // the ask rule: out of its session, a question is a notification and no sheet
        let sheet = app.staticTexts["ask-title"]
        let noSheet = ScreenRead.now { ScreenRead.isGone(sheet) }
        XCTAssertEqual(noSheet, .holds, "the question's sheet shows out of its session")
        setSwitches(["questions": false], in: app)
        expectDelivered("QA q-one?", isUp: false)
        shot("04-questions-off-took-it-down")
        setSwitches(["questions": true], in: app)
        expectDelivered("QA q-one?", isUp: true)
        shot("05-questions-on-posted-it-again")

        // every switch off: a question and a notice out of sight post nothing
        setSwitches(Dictionary(uniqueKeysWithValues: Self.kinds.map { ($0, false) }), in: app)
        enterTheSession(in: app)
        answer("QA q-one?", in: app)
        post(
            "ngtermctl notify qa-ntc-two --title QA; ngtermctl ask open 'QA q-two?' --button yes --button no",
            in: app
        )
        leaveTheSession(in: app)
        expectNoBanner(["qa-ntc-two", "QA q-two?"])
        expectDelivered("qa-ntc-two", isUp: false)
        expectDelivered("QA q-two?", isUp: false)
        shot("06-all-off-nothing-posted")
        // the question did reach the app: its sheet shows over its session
        enterTheSession(in: app)
        answer("QA q-two?", in: app)
        shot("07-all-off-question-was-open")
    }

    // MARK: the programs in the session

    // `command`, run by the session's shell after a delay and in the background, with the session and the instance's
    // control socket named
    @MainActor
    private func post(_ command: String, in app: XCUIApplication) {
        let terminal = app.textViews.firstMatch
        FakeApp.expect(terminal, "the session's terminal")
        terminal.tap()
        let line = command.components(separatedBy: "; ").map(Self.targeted).joined(separator: "; ")
        app.typeText("(sleep \(Self.delay); \(line)) >/dev/null 2>&1 &\n")
    }

    // the open question's sheet over its session, answered yes
    @MainActor
    private func answer(_ question: String, in app: XCUIApplication) {
        FakeApp.expect(app.staticTexts[question], "the sheet of \(question)", lasting: Self.wait)
        app.buttons["ask-button-yes"].tap()
        FakeApp.expectGone(app.staticTexts[question], "the sheet of \(question)", lasting: Self.offerWait)
    }

    // MARK: what SpringBoard shows

    @MainActor
    private func shown(_ text: String) -> XCUIElement {
        Self.springboard.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    @MainActor
    private func expectBanner(_ text: String) {
        FakeApp.expect(shown(text), "a banner of \(text)", lasting: Self.wait)
        shot("banner-\(text)")
    }

    @MainActor
    private func expectNoBanner(_ texts: [String]) {
        let polled = ScreenRead.Wait(lasting: Self.quiet).poll {
            for text in texts where ScreenRead.isShown(shown(text)) == .holds {
                return .holds
            }
            return .doesNotHold
        }
        if polled.held {
            shot("unexpected-banner")
        }
        XCTAssertFalse(polled.held, "a banner showed with every switch off")
    }

    // the delivered list holds `text` (`isUp`) or no longer does, within the wait
    @MainActor
    private func expectDelivered(_ text: String, isUp: Bool) {
        ScreenRead.expect("\(text) is\(isUp ? " not" : "") in the delivered list", lasting: Self.wait) {
            Self.deliveredHolds(text) == isUp
        }
        shot("delivered-\(isUp ? "holds" : "lacks")-\(text)")
    }

    // MARK: Settings

    // the switches `values` set in Settings > Notifications, read back, and Settings closed
    @MainActor
    private func setSwitches(_ values: [String: Bool], in app: XCUIApplication) {
        // a banner covers the top of a phone's screen, and a tap there opens its session
        FakeApp.expectGone(shown(Self.bannerMark), "a banner over the app", lasting: Self.wait)
        let open = app.buttons["settings"]
        if !open.waitForExistence(timeout: Self.offerWait) {
            leaveTheSession(in: app)
        }
        FakeApp.expect(open, "Settings")
        open.tap()
        FakeApp.expect(app.navigationBars["Settings"], "Settings")
        for kind in Self.kinds {
            guard let isOn = values[kind] else {
                continue
            }
            let row = app.switches["notice-\(kind)"]
            scroll(to: row, in: app)
            flip(row, to: isOn ? "1" : "0", in: app)
            XCTAssertEqual(row.value as? String, isOn ? "1" : "0", "\(kind) did not turn \(isOn ? "on" : "off")")
        }
        shot("settings-\(values.keys.sorted().joined(separator: "-"))")
        app.buttons["settings-done"].tap()
        FakeApp.expectGone(app.navigationBars["Settings"], "Settings")
    }

    @MainActor
    private func scroll(to element: XCUIElement, in app: XCUIApplication) {
        FormScroll.scroll(to: element, in: app.collectionViews["settings-form"], swipes: 12)
    }

    // presses the switch in `row` until it reads `value` (SettingsTests' flip says why a press)
    @MainActor
    private func flip(_ row: XCUIElement, to value: String, in app: XCUIApplication) {
        let flipped = NSPredicate(format: "value == %@", value)
        for _ in 0..<3 where ScreenRead.now({ ScreenRead.shows(row, matching: flipped) }) != .holds {
            scroll(to: row, in: app)
            row.switches.firstMatch.press(forDuration: 0.6)
            _ = ScreenRead.step(lasting: 3).poll { ScreenRead.shows(row, matching: flipped) }
        }
    }

    // MARK: sign in, the machine, the sessions

    // the welcome's Import a key, with the machine's key and its passphrase, to its public half, then Done
    @MainActor
    private func signIn(with machine: E2EMachine, in app: XCUIApplication) {
        let start = app.buttons["import-key"]
        FakeApp.expect(start, "the welcome's Import a key: is the install fresh?")
        start.tap()
        let field = app.textViews["private-key"]
        FakeApp.expect(field, "the private key's field")
        field.tap()
        field.typeText(machine.key.trimmingCharacters(in: .whitespacesAndNewlines))
        let passphrase = app.secureTextFields["passphrase"]
        passphrase.tap()
        passphrase.typeText(machine.passphrase + "\n")
        app.buttons["import-key-action"].tap()
        let page = app.descendants(matching: .any)["public-key-page"].firstMatch
        FakeApp.expect(page, "the imported key's public half")
        let done = app.buttons["key-done"]
        FakeApp.expect(done, "the public key page's Done")
        let closed = LogsPage.tap(
            done,
            until: { ScreenRead.isGone(page) },
            in: app,
            "the public key page stayed after Done"
        )
        LogsPage.closeLateOffer(unless: closed, in: app)
    }

    // the machine through the form, its host key trusted, connected
    @MainActor
    private func addMachine(_ machine: E2EMachine, in app: XCUIApplication) {
        FakeApp.showMachines(in: app)
        app.buttons["add-machine"].tap()
        E2EMachine.fill("machine-label", with: Self.label, in: app)
        E2EMachine.fill("machine-host", with: machine.host, in: app)
        E2EMachine.fill("machine-port", with: machine.port, in: app)
        E2EMachine.fill("machine-user", with: machine.user, in: app)
        app.buttons["machine-save"].tap()
        let trust = app.buttons["trust-host-key"]
        FakeApp.expect(trust, "the host key sheet")
        trust.tap()
        let row = NSPredicate(format: "identifier == %@ AND label CONTAINS 'Connected'", "machine-row-\(Self.label)")
        let connected = app.descendants(matching: .any).matching(row).firstMatch
        FakeApp.expect(connected, "Connected on the machine's row", lasting: Self.wait)
    }

    // New named session `name` on the machine, landing in its terminal; the first attach asks about notifying, and
    // `allowing` turns notifications on there and in the system's question
    @MainActor
    private func openANewSession(_ name: String, allowing: Bool, in app: XCUIApplication) {
        app.descendants(matching: .any)["machine-row-\(Self.label)"].firstMatch.tap()
        let new = app.buttons["new-session"]
        FakeApp.expect(new, "New session")
        let named = app.buttons["new-named-session"]
        // a press that lands while the list still moves opens no menu
        for _ in 0..<3 where !named.waitForExistence(timeout: Self.menuPress) {
            new.press(forDuration: Self.menuPress)
        }
        FakeApp.expect(named, "New named session in the button's menu")
        named.tap()
        let field = app.textFields["session-name"]
        FakeApp.expect(field, "the New session sheet")
        field.tap()
        field.typeText(name)
        app.buttons["create-session"].tap()
        let offer = app.buttons[allowing ? "notifications-allow" : "notifications-later"]
        if offer.waitForExistence(timeout: Self.offerWait) {
            offer.tap()
            if allowing {
                let allow = Self.springboard.alerts.buttons["Allow"]
                FakeApp.expect(allow, "the system's question about notifying")
                allow.tap()
            }
        } else if allowing {
            XCTFail("no question about notifying at the first attach")
        }
        FakeApp.expect(Self.title(of: name, in: app), "the session view of \(name)")
        let terminal = app.textViews.firstMatch
        ScreenRead.expect("the terminal of \(name) never showed a prompt", lasting: Self.wait) {
            ScreenRead.shows(terminal) { snapshot in
                !((snapshot.value as? String) ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            }
        }
    }

    // on an iPad, the other session made and the session on screen again, so that leaving it later is one tap
    @MainActor
    private func makeTheOtherSession(in app: XCUIApplication) {
        guard Self.isPad else {
            return
        }
        openANewSession(Self.other, allowing: false, in: app)
        enterTheSession(in: app)
    }

    // the session off the screen: a phone goes back to the machines; an iPad shows the other session in its place
    @MainActor
    private func leaveTheSession(in app: XCUIApplication) {
        if Self.isPad {
            let row = app.descendants(matching: .any)["session-\(Self.other)"].firstMatch
            FakeApp.expect(row, "the other session's row")
            row.tap()
            FakeApp.expect(Self.title(of: Self.other, in: app), "the other session's title")
            return
        }
        let add = app.buttons["add-machine"]
        for _ in 0..<3 where !add.exists {
            app.buttons["BackButton"].firstMatch.tap()
        }
        FakeApp.expect(add, "the machines")
    }

    // the session on screen again, through its machine's row on a phone
    @MainActor
    private func enterTheSession(in app: XCUIApplication) {
        let row = app.descendants(matching: .any)["session-\(Self.session)"].firstMatch
        if !row.exists {
            FakeApp.showMachines(in: app)
            app.descendants(matching: .any)["machine-row-\(Self.label)"].firstMatch.tap()
        }
        FakeApp.expect(row, "the session's row")
        row.tap()
        // after the app starts again, its earlier run still leads the pane
        let takeControl = app.buttons["Take control"]
        if takeControl.waitForExistence(timeout: Self.offerWait) {
            takeControl.tap()
            FakeApp.expectGone(takeControl, "Take control")
        }
        FakeApp.expect(Self.title(of: Self.session, in: app), "the session's title")
        FakeApp.expect(app.textViews.firstMatch, "the session's terminal")
    }

    // the whole screen: a banner shows over the app
    @MainActor
    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
