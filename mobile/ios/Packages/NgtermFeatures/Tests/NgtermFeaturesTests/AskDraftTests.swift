import NgtermCore
import Testing

@testable import NgtermFeatures

/// The draft and the answers follow ngterm-core's `AskDraft` and `PendingAsk::answer` (mobile/android's `AskDraftTest`
/// and its notification replies).
@Suite("an ask's draft")
internal struct AskDraftTests {
    @Test("a radio list starts on its first checked or first item")
    internal func radioStart() {
        let radio = TestAsks.choices(multiple: false, checked: [], other: nil)
        #expect(AskDraft(TestAsks.choosing(radio)).checked == ["a"])
        let checked = TestAsks.choices(multiple: false, checked: ["b", "c"], other: nil)
        #expect(AskDraft(TestAsks.choosing(checked)).checked == ["b"])
        let multiple = TestAsks.choices(multiple: true, checked: ["b", "c"], other: nil)
        #expect(AskDraft(TestAsks.choosing(multiple)).checked == ["b", "c"])
        let none = TestAsks.choices(multiple: true, checked: [], other: nil)
        #expect(AskDraft(TestAsks.choosing(none)).checked.isEmpty)
    }

    @Test("the free-form row takes a radio pick and gives it back when blank")
    internal func otherTakesThePick() {
        let ask = TestAsks.choosing(TestAsks.choices(multiple: false, checked: ["b"], other: "Something else"))
        let typed = AskDraft(ask).withText("d")
        #expect(typed.isOtherSelected)
        #expect(typed.selected.isEmpty)
        #expect(typed.otherText == "d")
        let cleared = typed.withText("  ")
        #expect(!cleared.isOtherSelected)
        #expect(cleared.selected == ["b"])
        #expect(cleared.otherText == nil)
    }

    @Test("the free-form row toggles only with text")
    internal func otherToggles() {
        let ask = TestAsks.choosing(TestAsks.choices(multiple: true, checked: [], other: "Other"))
        let blank = AskDraft(ask)
        #expect(blank.toggle(3) == blank)
        let typed = blank.withText("x")
        #expect(typed.isOtherSelected)
        #expect(!typed.toggle(3).isOtherSelected)
        #expect(typed.toggle(3).toggle(3).isOtherSelected)
    }

    @Test("a checkbox flips and a radio picks")
    internal func toggles() {
        let multiple = AskDraft(TestAsks.choosing(TestAsks.choices(multiple: true, checked: [], other: nil)))
        #expect(multiple.toggle(2).toggle(0).selected == ["a", "c"])
        #expect(multiple.toggle(2).toggle(0).toggle(0).selected == ["c"])
        let radio = AskDraft(TestAsks.choosing(TestAsks.choices(multiple: false, checked: [], other: "Other")))
            .withText("x")
        let picked = radio.toggle(1)
        #expect(picked.selected == ["b"])
        #expect(!picked.isOtherSelected)
        #expect(picked.toggle(9) == picked)
    }

    @Test("a button answers with the form, and the cancel button escapes")
    internal func answers() {
        let plain = TestAsks.plain()
        #expect(AskDraft(plain).reply(to: plain, button: "yes") == AskReply(button: "yes", text: nil, selected: nil))
        #expect(AskDraft(plain).reply(to: plain, button: "no") == AskDraft.escaped)

        let input = TestAsks.asking(RemoteAskInput(multiline: false, value: "main", placeholder: nil))
        #expect(AskDraft(input).reply(to: input, button: "yes") == AskReply(button: "yes", text: "main", selected: nil))
        #expect(
            AskDraft(input).withText("dev").reply(to: input, button: "yes")
                == AskReply(button: "yes", text: "dev", selected: nil))
        #expect(AskDraft(input).withText("dev").reply(to: input, button: "no") == AskDraft.escaped)

        let list = TestAsks.choosing(TestAsks.choices(multiple: true, checked: [], other: "Other"))
        let draft = AskDraft(list).toggle(1).withText("d")
        #expect(draft.reply(to: list, button: "yes") == AskReply(button: "yes", text: "d", selected: ["b"]))
    }

    @Test("text keeps to its field's rules")
    internal func textRules() {
        #expect(AskText.hasForbiddenControls("a\nb", multiline: false))
        #expect(!AskText.hasForbiddenControls("a\n\tb", multiline: true))
        #expect(AskText.hasForbiddenControls("a\u{7}", multiline: true))
        #expect(AskText.hasForbiddenControls("a\u{7f}", multiline: true))
        #expect(AskText.fit("a\nb", multiline: false) == "a b")
        #expect(AskText.fit("a\nb", multiline: true) == "a\nb")
        let long = String(repeating: "é", count: AskText.maxBytes)
        let clamped = AskText.clamp(long)
        #expect(clamped.utf8.count == AskText.maxBytes)
        #expect(AskText.clamp("ok") == "ok")
        #expect(AskDraft(TestAsks.plain()).withText(long).text.utf8.count == AskText.maxBytes)
    }

    @Test("a notification's reply answers with what was typed there, fitted, or the prefill")
    internal func notificationReplies() {
        let line = TestAsks.asking(RemoteAskInput(multiline: false, value: "main", placeholder: nil))
        #expect(
            AskDraft.notificationReply(line, button: "yes", typed: "a\nb")
                == AskReply(button: "yes", text: "a b", selected: nil))
        #expect(
            AskDraft.notificationReply(line, button: "yes", typed: nil)
                == AskReply(button: "yes", text: "main", selected: nil))
        #expect(AskDraft.notificationReply(line, button: "no", typed: "x") == AskDraft.escaped)
        let lines = TestAsks.asking(RemoteAskInput(multiline: true, value: "", placeholder: nil))
        #expect(
            AskDraft.notificationReply(lines, button: "yes", typed: "a\nb")
                == AskReply(button: "yes", text: "a\nb", selected: nil))
        let plain = TestAsks.plain()
        #expect(
            AskDraft.notificationReply(plain, button: "yes", typed: nil)
                == AskReply(button: "yes", text: nil, selected: nil))
    }
}
