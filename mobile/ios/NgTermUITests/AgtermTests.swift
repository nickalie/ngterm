import XCTest

/// A Mac running agterm on the fake core.
///
/// `studio`, whose one session `notes` runs Claude Code in its pane (the app's `FakeAgtermWorld`). Its row, its
/// sessions screen and its session view say it runs agterm; its sessions list, attach and end as an ngterm machine's
/// do; and its pane keeps showing the terminal: agterm has no chat view, so Chat view is off and takes no tap, and no
/// view switch offers a chat.
internal final class AgtermTests: XCTestCase {
    // how long a view that should not come is watched for
    private static let steady: TimeInterval = 3

    @MainActor
    private static func any(_ identifier: String, in app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any)[identifier].firstMatch
    }

    @MainActor
    private static func labelled(
        _ identifier: String,
        containing text: String,
        in app: XCUIApplication
    ) -> XCUIElement {
        app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier == %@ AND label CONTAINS %@", identifier, text))
            .firstMatch
    }

    @MainActor
    internal func testAnAgtermMacSaysSoAndItsPaneKeepsTheTerminal() throws {
        let app = FakeApp.launch(withKey: true, arguments: ["-ngterm-fake-agterm"], for: self)
        FakeApp.showMachines(in: app)
        FakeApp.expectRow("studio", reads: "Connected", in: app)
        FakeApp.expect(Self.labelled("machine-row-studio", containing: "agterm", in: app), "the row's agterm badge")
        try FakeApp.audit(app)

        Self.any("machine-row-studio", in: app).tap()
        let row = Self.any("session-notes", in: app)
        FakeApp.expect(row, "the session's row")
        FakeApp.expect(
            FakeApp.element(containing: "studio, nick at studio.lan, port 22, agterm", in: app),
            "the title's agterm badge"
        )
        let note = Self.labelled("sessions-peer-note", containing: "the agent chat view", in: app)
        FakeApp.expect(note, "what agterm lacks, under the list")
        try FakeApp.audit(app)

        row.tap()
        FakeApp.expect(Self.labelled("session-title", containing: "agterm", in: app), "the session's agterm badge")
        let screen = app.textViews.matching(NSPredicate(format: "value CONTAINS %@", "agterm$")).firstMatch
        FakeApp.expect(screen, "the pane's terminal")
        let chat = Self.any("chat", in: app)
        let viewSwitch = Self.any("session-view-switch", in: app)
        XCTAssertFalse(chat.waitForExistence(timeout: Self.steady), "the pane showed a chat")
        XCTAssertFalse(viewSwitch.exists, "a view switch offered a chat")
        try FakeApp.audit(app)

        app.buttons["session-menu"].tap()
        let chatView = Self.any("session-chat-view", in: app)
        FakeApp.expect(chatView, "Chat view in More")
        XCTAssertFalse(chatView.isEnabled, "Chat view took a tap on agterm")
        let end = app.buttons["session-end"]
        end.tap()
        FakeApp.expect(FakeApp.element(containing: "End notes?", in: app), "End session confirmation")
        let confirm = app.buttons.matching(identifier: "End session").allElementsBoundByIndex.first(where: \.isHittable)
        try XCTUnwrap(confirm, "no End session in the confirmation").tap()
        FakeApp.expect(FakeApp.element(containing: "No sessions on studio", in: app), "the list without the session")
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
