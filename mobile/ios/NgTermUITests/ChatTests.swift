import XCTest

/// The agent chat on the fake core's `split`, whose session `work` runs Claude Code in its first pane (the app's
/// `FakeChatWorld`): what the user types reaches the agent and what it says shows, its questions and permission prompts
/// come as cards and are answered from them, a card answered while the link was down goes, and the session's Chat view
/// holds as set.
internal final class ChatTests: XCTestCase {
    private static let wait: TimeInterval = 15
    // how long a view that should stay is watched for
    private static let steady: TimeInterval = 3
    private static let beat: TimeInterval = 0.25

    @MainActor
    private static func chat(in app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any)["chat"].firstMatch
    }

    @MainActor
    private static func field(in app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any)["chat-prompt"].firstMatch
    }

    // types `text` into the prompt and sends it
    @MainActor
    private static func send(_ text: String, in app: XCUIApplication) {
        let field = field(in: app)
        FakeApp.expect(field, "the prompt field")
        field.tap()
        field.typeText(text)
        app.buttons["chat-send"].tap()
    }

    @MainActor
    private static func expectGone(_ element: XCUIElement, _ what: String) {
        XCTAssertTrue(element.waitForNonExistence(timeout: wait), "\(what) stayed")
    }

    @MainActor
    private static func option(_ label: String, in app: XCUIApplication) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", label)).firstMatch
    }

    // the app on `work`'s session view, its first pane's chat open
    @MainActor
    private func openChat() -> XCUIApplication {
        let app = FakeApp.launch(withKey: true, arguments: ["-ngterm-fake-session", "-ngterm-fake-chat"], for: self)
        FakeApp.showMachines(in: app)
        FakeApp.expectRow("split", reads: "Connected", in: app)
        app.descendants(matching: .any)["machine-row-split"].firstMatch.tap()
        let row = app.descendants(matching: .any)["session-work"].firstMatch
        FakeApp.expect(row, "the session's row")
        row.tap()
        FakeApp.expect(Self.chat(in: app), "the first pane's chat")
        FakeApp.expect(FakeApp.element(containing: "The guide is in docs/install.md.", in: app), "the catch-up's reply")
        return app
    }

    @MainActor
    internal func testPromptsReachTheAgentAndTheNextIsTypedWhileOneIsOnItsWay() {
        let app = openChat()
        Self.send("Add a troubleshooting section, take your time", in: app)
        let sending = app.buttons["Sending"]
        FakeApp.expect(sending, "Send's spinner")
        app.typeText("Then check the links")
        XCTAssertTrue(sending.exists, "the first prompt was no longer on its way as the next was typed")
        FakeApp.expect(
            FakeApp.element(containing: "Done: Add a troubleshooting section, take your time.", in: app),
            "the reply"
        )
        FakeApp.expect(
            FakeApp.element(containing: "You: Add a troubleshooting section, take your time", in: app),
            "the prompt's row"
        )
        app.buttons["chat-send"].tap()
        FakeApp.expect(FakeApp.element(containing: "You: Then check the links", in: app), "the typed prompt's row")
        FakeApp.expect(FakeApp.element(containing: "Done: Then check the links.", in: app), "the second reply")
    }

    @MainActor
    internal func testALongReplyIsFollowedToItsEnd() {
        let app = openChat()
        Self.send("write a lot", in: app)
        let last = app.staticTexts["Paragraph 30 ends here."]
        FakeApp.expect(last, "the reply's last paragraph")
        let field = Self.field(in: app)
        let list = Self.chat(in: app)
        // a selectable text's hit test lands on its container, so where it lies on screen is what tells
        for _ in 0..<Int(Self.wait / Self.beat) {
            if last.frame.minY >= list.frame.minY, last.frame.maxY <= field.frame.minY {
                return
            }
            _ = XCTWaiter.wait(for: [XCTestExpectation(description: "a beat")], timeout: Self.beat)
        }
        XCTFail("the list stayed above the reply: its last paragraph at \(last.frame), the prompt at \(field.frame)")
    }

    @MainActor
    internal func testClaudesQuestionIsAQuestionCardAnsweredThereAndAskedAgain() {
        let app = openChat()
        Self.send("ask me", in: app)
        let send = app.buttons["Send answers"]
        FakeApp.expect(send, "the question card")
        FakeApp.expect(FakeApp.element(containing: "Which shell should the guide use?", in: app), "the question")
        XCTAssertFalse(FakeApp.element(containing: "Allow AskUserQuestion", in: app).exists, "an approval card asked")
        Self.option("fish", in: app).tap()
        send.tap()
        FakeApp.expect(FakeApp.element(containing: "Going with fish.", in: app), "the reply to the answer")
        Self.expectGone(send, "the answered card")

        // the same question again gets its card, though the answered one asked the same
        Self.send("ask me", in: app)
        FakeApp.expect(send, "the question card asked again")
        Self.option("bash", in: app).tap()
        send.tap()
        FakeApp.expect(FakeApp.element(containing: "Going with bash.", in: app), "the reply to the second answer")
        Self.expectGone(send, "the second answered card")
    }

    @MainActor
    internal func testAPermissionPromptIsAllowedFromItsCard() {
        let app = openChat()
        Self.send("run the build", in: app)
        FakeApp.expect(FakeApp.element(containing: "Allow Bash?", in: app), "the approval card")
        let yes = app.buttons["Yes"]
        FakeApp.expect(yes, "the dialog's Yes")
        yes.tap()
        FakeApp.expect(FakeApp.element(containing: "The build passed.", in: app), "the reply after the build")
        Self.expectGone(FakeApp.element(containing: "Allow Bash?", in: app), "the allowed card")
    }

    @MainActor
    internal func testACardAnsweredWhileTheLinkWasDownGoesAndSendWorksAgain() {
        let app = openChat()
        Self.send("ask me, then drop", in: app)
        let send = app.buttons["Send answers"]
        FakeApp.expect(send, "the question card")
        FakeApp.expect(FakeApp.element(containing: "Reconnecting to", in: app), "the reconnecting strip")
        Self.expectGone(send, "the card answered in the terminal")
        Self.expectGone(FakeApp.element(containing: "Reconnecting to", in: app), "the reconnecting strip")
        Self.send("carry on", in: app)
        FakeApp.expect(FakeApp.element(containing: "Done: carry on.", in: app), "the reply after the reconnect")
    }

    @MainActor
    internal func testTheSessionsChatViewHoldsAsSet() {
        let app = openChat()
        let chat = Self.chat(in: app)
        app.buttons["session-menu"].tap()
        let toggle = app.descendants(matching: .any)["session-chat-view"].firstMatch
        FakeApp.expect(toggle, "Chat view in the session's menu")
        toggle.tap()
        Self.expectGone(chat, "the chat with Chat view off")
        FakeApp.expect(
            app.textViews.matching(NSPredicate(format: "value CONTAINS 'P1 ready'")).firstMatch,
            "the first pane's terminal"
        )
        app.buttons["session-menu"].tap()
        FakeApp.expect(toggle, "Chat view in the session's menu again")
        toggle.tap()
        FakeApp.expect(chat, "the chat with Chat view on again")
        let stays = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: chat)
        XCTAssertEqual(XCTWaiter.wait(for: [stays], timeout: Self.steady), .timedOut, "the chat flipped back")
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
