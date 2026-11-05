import NgtermCore
import SwiftUI
import Testing

@testable import NgtermFeatures

/// Goldens of the ask sheets (a button ask with every kind of button, an input ask, a radio list with its free-form row
/// picked, a checkbox list) and of the question about notifying.
@MainActor
@Suite("ask sheet snapshots")
internal struct AskSnapshotTests {
    private static let buttons = RemoteAsk(
        id: "deploy",
        title: "Deploy api to production?",
        message: "The tests passed on main. The last deploy was 2 days ago.",
        buttons: [
            RemoteAskButton(id: "deploy", label: "Deploy", hotkey: nil),
            RemoteAskButton(id: "diff", label: "Show the diff", hotkey: nil),
            RemoteAskButton(id: "rollback", label: "Roll back", hotkey: nil),
            RemoteAskButton(id: "later", label: "Not now", hotkey: nil),
        ],
        defaultId: "deploy",
        destructiveId: "rollback",
        cancelId: "later",
        style: "terminal",
        align: "right",
        width: nil,
        pane: nil,
        input: nil,
        choices: nil
    )

    private static func ignore(_: AskReply) {
        // a golden answers nothing
    }

    private static func question(_ ask: RemoteAsk, _ draft: AskDraft) -> some View {
        AskQuestion(ask: ask, draft: .constant(draft), onAnswer: ignore)
    }

    @Test(
        "a button ask: the default, another, a destructive one and the cancel button", arguments: ScreenVariant.allCases
    )
    internal func buttonAsk(variant: ScreenVariant) {
        let question = Self.question(Self.buttons, AskDraft(Self.buttons))
        ScreenSnapshot.check(question, named: "ask-buttons", variant: variant)
    }

    @Test("an input ask with its prefill", arguments: [ScreenVariant.light, .dark])
    internal func inputAsk(variant: ScreenVariant) {
        var ask = TestAsks.ask(["create", "no"], input: nil, choices: nil)
        ask.title = "Name the branch"
        ask.message = "It is made from main."
        ask.buttons = [
            RemoteAskButton(id: "create", label: "Create", hotkey: nil),
            RemoteAskButton(id: "no", label: "Cancel", hotkey: nil),
        ]
        ask.input = RemoteAskInput(multiline: false, value: "fix-parser", placeholder: "Branch")
        ScreenSnapshot.check(Self.question(ask, AskDraft(ask)), named: "ask-input", variant: variant)
    }

    @Test("a radio list with its free-form row typed and picked", arguments: ScreenVariant.allCases)
    internal func radioAsk(variant: ScreenVariant) {
        var ask = TestAsks.choosing(TestAsks.choices(multiple: false, checked: ["b"], other: "Another stage"))
        ask.title = "Where should it go?"
        let draft = AskDraft(ask).withText("staging-2")
        ScreenSnapshot.check(Self.question(ask, draft), named: "ask-radio", variant: variant)
    }

    @Test("a checkbox list with subtitles")
    internal func checkboxAsk() {
        let list = RemoteAskChoices(
            choices: [
                RemoteAskChoice(id: "lint", label: "Lint", subtitle: "swiftlint and swift-format"),
                RemoteAskChoice(id: "unit", label: "Unit tests", subtitle: nil),
                RemoteAskChoice(id: "ui", label: "UI tests", subtitle: "on both simulators"),
            ],
            multiple: true,
            checked: ["lint", "unit"],
            other: nil
        )
        var ask = TestAsks.choosing(list)
        ask.title = "Which checks should run?"
        ScreenSnapshot.check(Self.question(ask, AskDraft(ask)), named: "ask-checkboxes", variant: .light)
    }

    @Test("the question about notifying", arguments: ScreenVariant.allCases)
    internal func notificationQuestion(variant: ScreenVariant) {
        let question = NotificationPermissionQuestion(onAllow: ScreenSnapshot.ignore, onLater: ScreenSnapshot.ignore)
        ScreenSnapshot.check(question, named: "notification-question", variant: variant)
    }
}
