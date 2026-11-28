import XCTest

/// The app on the fake core (or on its real one, for a test on a real machine), as the UI tests launch and drive it.
internal enum FakeApp {
    internal static let stillArgument = "-ngterm-no-animations"
    internal static let textSizeArgument = "-ngterm-text-size-at-once"

    // how long a screen may take to come
    private static let wait: TimeInterval = 30
    // how long a list or the navigation bars' buttons are watched for, at most, before an audit: this many reads, this
    // far apart
    private static let settleReads = 20
    private static let settleBeat: TimeInterval = 0.25
    // how many reads running a list (its text inside its rows) or the bars' buttons have to keep still for
    private static let settleQuietReads = 3
    // how often a list whose text lies outside its rows is nudged, and how far
    private static let settleNudges = 3
    private static let nudgeDistance: CGFloat = 12
    private static let nudgePress: TimeInterval = 0.05
    // where in a list a nudge starts: its middle
    private static let middle: CGFloat = 0.5
    // the audit's checks that make the app's text larger, and so lay its screen out again while they run
    private static let resizingAudits: XCUIAccessibilityAuditType = [.dynamicType, .textClipped]

    /// Starts the app on the fake core, with a key when `withKey` is true.
    ///
    /// An iPad stays on its side between tests, so its three columns stay beside each other.
    /// After the app starts, an upright window turns on its side before an audit measures its sheets.
    @MainActor
    internal static func launch(withKey: Bool, for test: XCTestCase) -> XCUIApplication {
        launch(withKey: withKey, arguments: [], for: test)
    }

    /// Starts the app as ``launch(withKey:for:)`` does, with the extra arguments `arguments`.
    @MainActor
    internal static func launch(withKey: Bool, arguments: [String], for _: XCTestCase) -> XCUIApplication {
        start(["-ngterm-fake-core"] + (withKey ? ["-ngterm-fake-key"] : []) + arguments)
    }

    /// Starts the app on its real core, with the iPad orientation from ``launch(withKey:for:)``.
    ///
    /// The app uses the simulator's stored data, as ``LogsOnAMachineTests`` does after
    /// `scripts/test-ios-e2e.sh` installs it.
    @MainActor
    internal static func launchOnTheRealCore(for _: XCTestCase) -> XCUIApplication {
        start([])
    }

    @MainActor
    private static func start(_ arguments: [String]) -> XCUIApplication {
        let isPad = UIDevice.current.userInterfaceIdiom == .pad
        let app = XCUIApplication()
        // the app's view animations off (AppDelegate.stillArgument): on the iPad simulator a system animation
        // sometimes never ends, and XCTest then waits 2 x 60 s at each action for it. And a text size change laid out
        // at once (AppDelegate.textSizeArgument): a Dynamic Type audit reads each size about 0.1 s after it sets it.
        app.launchArguments = arguments + [stillArgument, textSizeArgument]
        app.launch()
        if isPad {
            expectLandscape(app)
        }
        return app
    }

    // A turn before the app starts can leave its window upright. The other side forces a new turn after it starts.
    @MainActor
    private static func expectLandscape(_ app: XCUIApplication) {
        let window = app.windows.firstMatch
        let frame = window.frame
        if frame.width > frame.height {
            return
        }
        XCUIDevice.shared.orientation =
            XCUIDevice.shared.orientation == .landscapeLeft ? .landscapeRight : .landscapeLeft
        if !expectShape(of: window, landscape: true) {
            XCTFail("the app's window did not turn on its side: \(window.frame)")
        }
    }

    // reads `window` a beat apart until it is on its side (`landscape`: wider than it is tall) or upright; false when
    // it does not come to it. A read that fails gives no frame, so it is not one of the reads
    @MainActor
    internal static func expectShape(of window: XCUIElement, landscape: Bool) -> Bool {
        let shape = ScreenRead.Wait(reads: Int(wait / settleBeat), beat: settleBeat).poll {
            ScreenRead.shows(window) { snapshot in
                let frame = snapshot.frame
                return landscape ? frame.width > frame.height : frame.height > frame.width
            }
        }
        return shape.held
    }

    /// Waits for `element`, failing the test when it doesn't come.
    ///
    /// The wait reads the screen again and again, a beat apart, and gives up after a number of reads, not after a short
    /// time alone: on a loaded runner one read can take as long as a whole timed wait. On CI a 30 s wait for the chat
    /// (ChatTests) read the screen twice, its first read took 28 s, and the chat came 2 s after the wait ended. A
    /// time limit of some minutes stops the wait before all its reads when each read is that slow (``ScreenRead``).
    @MainActor
    internal static func expect(_ element: XCUIElement, _ what: String) {
        expect(element, what, lasting: 0)
    }

    /// Waits for `element` as ``expect(_:_:)`` does, and for `lasting` seconds at least.
    @MainActor
    internal static func expect(_ element: XCUIElement, _ what: String, lasting: TimeInterval) {
        ScreenRead.expect("no \(what)", lasting: lasting) { ScreenRead.isShown(element) }
    }

    /// Waits until `element` has gone, as ``expect(_:_:)`` waits for one to come.
    ///
    /// Only a read that completed and found no such element tells it has gone: a read that fails also says that the
    /// element does not exist, and on CI one read failed after 9 s while the element still showed.
    @MainActor
    internal static func expectGone(_ element: XCUIElement, _ what: String) {
        expectGone(element, what, lasting: 0)
    }

    /// Waits until `element` has gone as ``expectGone(_:_:)`` does, and for `lasting` seconds at least.
    @MainActor
    internal static func expectGone(_ element: XCUIElement, _ what: String, lasting: TimeInterval) {
        ScreenRead.expect("\(what) stayed", lasting: lasting) { ScreenRead.isGone(element) }
    }

    /// Attaches the app's hierarchy and a screenshot to the test, in an activity named `name`, for a failure's triage.
    @MainActor
    internal static func attachScreen(_ name: String) {
        XCTContext.runActivity(named: name) { activity in
            let tree = XCTAttachment(string: XCUIApplication().debugDescription)
            tree.name = "hierarchy"
            tree.lifetime = .keepAlways
            activity.add(tree)
            let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
            shot.name = "screen"
            shot.lifetime = .keepAlways
            activity.add(shot)
        }
    }

    /// Tells the app's fake worlds the test has seen what the cue `name` (the app's `FakeCues.Cue`) waits on.
    ///
    /// Posts of one name can coalesce into one: post `name` again only after seeing what its previous post let on.
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
            try auditForm(app, page: form)
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
        expectGone(app.keyboards.firstMatch, "the keyboard", lasting: wait)
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

    /// Waits until the buttons of the navigation bars have kept still for some reads running, before an audit measures
    /// them.
    ///
    /// When a phone goes back, the navigation bar's Liquid Glass turns the leaving screen's button into the coming
    /// screen's, and the app's switch for no animations does not stop that: an audit during the change measured a
    /// button too small to touch, and the button it named had gone when the test looked for it. The button of the
    /// change can have no identifier, so the wait reads every button of the bars, not one. When they do not keep
    /// still, the audit has its say.
    @MainActor
    internal static func settleBars(in app: XCUIApplication) {
        var before: [CGRect] = []
        var quiet = 0
        ScreenRead.Wait(reads: settleReads, beat: settleBeat).poll {
            ScreenRead.shows(app) { snapshot in
                let now = barButtonFrames(under: snapshot)
                quiet = !now.isEmpty && now == before ? quiet + 1 : 0
                before = now
                return quiet >= settleQuietReads
            }
        }
    }

    @MainActor
    private static func barButtonFrames(under snapshot: any XCUIElementSnapshot) -> [CGRect] {
        guard snapshot.elementType == .navigationBar else {
            return snapshot.children.flatMap { barButtonFrames(under: $0) }
        }
        return buttonFrames(under: snapshot)
    }

    @MainActor
    private static func buttonFrames(under snapshot: any XCUIElementSnapshot) -> [CGRect] {
        let own = snapshot.elementType == .button ? [snapshot.frame] : []
        return own + snapshot.children.flatMap { buttonFrames(under: $0) }
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

    /// Audits the form `page` for everything but Dynamic Type, setting aside what ``audit(_:)`` does, and the contrast
    /// and the clipping of text the page cuts at its edge: on an iPad's sheet a form's last rows can run past its foot.
    ///
    /// The form is a list, which lets go of the rows the audit's larger text pushes off the screen: it then reports
    /// them as not scaling, and measures their contrast while the list lays them out again. Every font of the form is a
    /// Dynamic Type style, and its golden at an accessibility text size (MachinesSnapshotTests) shows them scaling.
    ///
    /// Text cut at the page's edge has its contrast measured against what lies past the edge, as
    /// ``auditSheet(_:page:)`` says. Text that reaches the page's foot (an iPad's sheet scrolled to its end shows the
    /// last footer flush with it) runs past the foot at the audit's larger text, which reports it clipped: that is the
    /// page's scroll edge, not the text's own frame. Every text of the form wraps with no line limit, and the user
    /// scrolls to read the rest (the same golden shows it).
    @MainActor
    internal static func auditForm(_ app: XCUIApplication, page: XCUIElement) throws {
        let shown = page.frame
        try performAudit(of: app, for: XCUIAccessibilityAuditType.all.subtracting(.dynamicType)) { issue in
            if let element = issue.element, issue.auditType.isCutAtFoot(element.frame, by: shown) {
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

extension XCUIAccessibilityAuditType {
    // a point's slack for the frames' rounding
    private static let footSlack: CGFloat = 1

    // whether the page `shown` cuts the text at `frame` at its foot: its contrast once it runs past the page, its
    // clipping once it reaches the foot
    internal func isCutAtFoot(_ frame: CGRect, by shown: CGRect) -> Bool {
        switch self {
        case .contrast:
            !shown.contains(frame)

        case .textClipped:
            frame.maxY >= shown.maxY - Self.footSlack

        default:
            false
        }
    }
}
