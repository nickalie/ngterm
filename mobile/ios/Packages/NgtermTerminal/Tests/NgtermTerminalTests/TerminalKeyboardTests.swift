import NgtermCore
import NgtermCoreFakes
import NgtermTerminal
import Testing
import UIKit

/// The soft keyboard, its composition and the extra keys against the fake pane, which records the input
/// (mobile/android's `TerminalKeyboardTest`).
@MainActor
@Suite("the soft keyboard and the extra keys")
internal struct TerminalKeyboardTests {
    // a rig whose view is its keyboard's target, as the first responder would be
    private static func rig() -> TerminalRig {
        let rig = TerminalRig(clipboard: nil)
        rig.view.keyboard.target = rig.view
        return rig
    }

    private static func press(_ name: String, _ mods: UInt8 = 0) -> PaneKey {
        KeyMapping.named(name, mods: mods)
    }

    // the keys a pane took, presses and repeats only
    private static func presses(_ inputs: [FakeInput]) -> [PaneKey] {
        inputs.compactMap { input in
            if case .key(let key) = input, key.action != .release {
                return key
            }
            return nil
        }
    }

    // the input a pane took, its key releases left out
    private static func withoutReleases(_ inputs: [FakeInput]) -> [FakeInput] {
        inputs.filter { input in
            if case .key(let key) = input {
                return key.action != .release
            }
            return true
        }
    }

    @Test("it asks for plain text: no autocorrect, spell check, smart punctuation, capitals or predictions")
    internal func traits() {
        let view = Self.rig().view
        #expect(view.autocorrectionType == .no)
        #expect(view.spellCheckingType == .no)
        #expect(view.smartQuotesType == .no)
        #expect(view.smartDashesType == .no)
        #expect(view.smartInsertDeleteType == .no)
        #expect(view.autocapitalizationType == .none)
        #expect(view.inlinePredictionType == .no)
        #expect(view.writingToolsBehavior == .none)
        #expect(view.hasText, "the keyboard's delete key always sends")
    }

    @Test("a tap opens the soft keyboard; a click, a long press or the menu only take the focus")
    internal func softKeyboard() {
        let rig = Self.rig()
        let view = rig.view
        #expect(!view.softKeyboardWanted)
        #expect(view.inputView != nil, "no keys while the keyboard isn't wanted")
        #expect(view.inputAccessoryView === view.keyboard.bar)
        view.pointerGestures.press(.began, at: rig.point(2, 1), buttons: .primary, mods: [], time: 1)
        view.pointerGestures.press(.ended, at: rig.point(2, 1), buttons: .primary, mods: [], time: 1)
        #expect(!view.softKeyboardWanted)
        view.gestures.tap(at: rig.point(2, 1), time: 5)
        #expect(view.softKeyboardWanted)
        #expect(view.inputView == nil, "the soft keyboard")
        view.hideSoftKeyboard()
        #expect(!view.softKeyboardWanted)
        #expect(view.inputView != nil)
        view.keyboard.bar.press(.keyboard)
        #expect(view.softKeyboardWanted, "the bar's keyboard key opens it again")
    }

    @Test("the extra keys stand aside for a hardware keyboard")
    internal func hardwareKeyboard() {
        let hardware = HardwareKeyboard(fixed: false)
        let rig = TerminalRig(clipboard: nil, keyboard: TerminalKeyboard(hardware: hardware))
        #expect(rig.view.inputAccessoryView != nil)
        hardware.update(attached: true)
        #expect(rig.view.inputAccessoryView == nil)
        hardware.update(attached: false)
        #expect(rig.view.inputAccessoryView != nil)
    }

    @Test("the soft keyboard types what it commits, its Return as Enter and its delete as backspace")
    internal func commits() {
        let rig = Self.rig()
        let view = rig.view
        view.insertText("ls -la")
        view.insertText("\n")
        view.insertText("echo hi\n")
        view.deleteBackward()
        #expect(
            rig.inputs == [
                .typed("ls -la"),
                .key(Self.press("enter")),
                .key(KeyMapping.named("enter", mods: 0, action: .release)),
                .typed("echo hi"),
                .key(Self.press("enter")),
                .key(KeyMapping.named("enter", mods: 0, action: .release)),
                .key(Self.press("backspace")),
                .key(KeyMapping.named("backspace", mods: 0, action: .release)),
            ]
        )
    }

    @Test("a composition waits at the cursor until it commits")
    internal func composition() {
        let rig = Self.rig()
        let view = rig.view
        view.setMarkedText("ni", selectedRange: NSRange(location: 2, length: 0))
        #expect(view.composition == "ni")
        #expect(view.markedTextRange != nil)
        view.setMarkedText("に", selectedRange: NSRange(location: 1, length: 0))
        #expect(view.composition == "に")
        #expect(rig.inputs.isEmpty, "nothing reaches the program while composing")
        let frame = view.compositionFrame
        #expect(frame.minX == view.font.left(0))
        #expect(frame.minY == view.font.top(0))
        #expect(frame.height == view.font.cellHeight)
        #expect(frame.width > 0)
        view.insertText("日本")
        #expect(view.composition.isEmpty)
        #expect(view.markedTextRange == nil)
        view.setMarkedText("語", selectedRange: NSRange(location: 1, length: 0))
        view.unmarkText()
        #expect(rig.inputs == [.typed("日本"), .typed("語")])
        #expect(view.composition.isEmpty)
    }

    @Test("a deletion takes from the composition, and past it is the program's backspace")
    internal func deletions() {
        let rig = Self.rig()
        let view = rig.view
        view.setMarkedText("ab", selectedRange: NSRange(location: 2, length: 0))
        view.deleteBackward()
        #expect(view.composition == "a")
        #expect(rig.inputs.isEmpty, "the composition held what was deleted")
        view.deleteBackward()
        view.deleteBackward()
        #expect(view.composition.isEmpty)
        #expect(Self.presses(rig.inputs) == [Self.press("backspace")])
        // an emoji goes whole
        view.setMarkedText("a\u{1F600}", selectedRange: NSRange(location: 3, length: 0))
        view.deleteBackward()
        #expect(view.composition == "a")
    }

    @Test("the extra keys send their keys, and type their characters")
    internal func extraKeys() {
        let rig = Self.rig()
        let bar = rig.view.keyboard.bar
        for key in [ExtraKey.escape, .tab, .pageDown, .pipe] {
            bar.press(key)
            bar.release(key)
        }
        #expect(Self.presses(rig.inputs) == [Self.press("escape"), Self.press("tab"), Self.press("page_down")])
        #expect(rig.inputs.last == .typed("|"))
    }

    @Test("a sticky Ctrl holds for the next key, and Alt locks")
    internal func sticky() {
        let rig = Self.rig()
        let keyboard = rig.view.keyboard
        let bar = keyboard.bar
        bar.press(.control)
        #expect(keyboard.control == .once)
        rig.view.insertText("c")
        #expect(Self.presses(rig.inputs) == [KeyMapping.forText("c", mods: KeyMapping.control)])
        #expect(keyboard.control == .off, "a once-held Ctrl lets go after one key")

        let before = rig.inputs.count
        bar.press(.alt)
        bar.press(.alt)
        #expect(keyboard.alt == .locked)
        bar.press(.upArrow)
        bar.release(.upArrow)
        bar.press(.slash)
        #expect(
            Self.presses(rig.inputs(after: before)) == [
                Self.press("arrow_up", KeyMapping.alt), KeyMapping.forText("/", mods: KeyMapping.alt),
            ]
        )
        #expect(keyboard.alt == .locked, "a locked Alt stays")
        bar.press(.alt)
        #expect(keyboard.alt == .off)
    }

    @Test("hiding the extra keys lets go of a locked Ctrl and Alt, whose keys go with them")
    internal func hidingLetsGo() {
        let rig = Self.rig()
        let keyboard = rig.view.keyboard
        keyboard.bar.press(.control)
        keyboard.bar.press(.control)
        keyboard.bar.press(.alt)
        #expect(keyboard.control == .locked && keyboard.alt == .once)
        keyboard.layout = .hidden
        #expect(keyboard.control == .off && keyboard.alt == .off)
        let before = rig.inputs.count
        rig.view.insertText("c")
        #expect(rig.inputs(after: before) == [.typed("c")], "typed as it is, not as Ctrl-C")
    }

    @Test("a held arrow repeats, and Esc doesn't")
    internal func repeats() async throws {
        let rig = Self.rig()
        let bar = rig.view.keyboard.bar
        bar.press(.leftArrow)
        try await Task.sleep(for: .milliseconds(700))
        bar.release(.leftArrow)
        let lefts = Self.presses(rig.inputs).filter { $0.key == "arrow_left" }.count
        #expect(lefts >= 3, "one press, then a repeat every 50 ms after 400 ms: \(lefts)")
        let before = rig.inputs.count
        try await Task.sleep(for: .milliseconds(200))
        #expect(rig.inputs.count == before, "it stops when let go")
        bar.press(.escape)
        try await Task.sleep(for: .milliseconds(600))
        bar.release(.escape)
        #expect(Self.presses(rig.inputs(after: before)) == [Self.press("escape")])
    }

    @Test("a bar UIKit keeps after its keyboard is gone does nothing")
    internal func barOutlivesKeyboard() {
        var keyboard: TerminalKeyboard? = TerminalKeyboard(hardware: HardwareKeyboard(fixed: false))
        let bar = keyboard?.bar
        weak let gone = keyboard
        keyboard = nil
        #expect(gone == nil, "the keyboard went")
        bar?.press(.leftArrow)
        bar?.release(.leftArrow)
        bar?.refresh()
        bar?.layoutIfNeeded()
        #expect(bar?.rows == ExtraKey.twoRows)
    }

    @Test("a key let go stops its own repeat, not that of a key held with it")
    internal func repeatsOfOthers() async throws {
        let rig = Self.rig()
        let bar = rig.view.keyboard.bar
        let lefts = { Self.presses(rig.inputs).filter { $0.key == "arrow_left" }.count }
        let rights = { Self.presses(rig.inputs).filter { $0.key == "arrow_right" }.count }
        bar.press(.leftArrow)
        bar.press(.slash)
        bar.release(.slash)
        try await Task.sleep(for: .milliseconds(600))
        #expect(lefts() >= 2, "a tap of / beside it left the arrow repeating: \(lefts())")
        // a second arrow takes the repeat over, and letting go of the first leaves it
        bar.press(.rightArrow)
        bar.release(.leftArrow)
        try await Task.sleep(for: .milliseconds(600))
        bar.release(.rightArrow)
        let held = lefts()
        #expect(rights() >= 2, "the right arrow repeats on: \(rights())")
        try await Task.sleep(for: .milliseconds(200))
        #expect(lefts() == held, "the left arrow stopped as the right one took over")
    }

    @Test("a key from elsewhere commits the composition first")
    internal func commitsFirst() {
        let rig = Self.rig()
        let view = rig.view
        let bar = view.keyboard.bar
        view.setMarkedText("ls", selectedRange: NSRange(location: 2, length: 0))
        bar.press(.tab)
        #expect(Self.withoutReleases(rig.inputs) == [.typed("ls"), .key(Self.press("tab"))])
        #expect(view.composition.isEmpty)
        let before = rig.inputs.count
        view.setMarkedText("x", selectedRange: NSRange(location: 1, length: 0))
        bar.press(.pipe)
        #expect(rig.inputs(after: before) == [.typed("x"), .typed("|")])
    }

    @Test("a sticky modifier armed while composing is for the key after")
    internal func stickyWhileComposing() {
        let rig = Self.rig()
        let view = rig.view
        let keyboard = view.keyboard
        view.setMarkedText("ls", selectedRange: NSRange(location: 2, length: 0))
        keyboard.bar.press(.control)
        #expect(rig.inputs == [.typed("ls")], "the composition went out plain")
        #expect(keyboard.control == .once)
        keyboard.bar.press(.leftArrow)
        keyboard.bar.release(.leftArrow)
        #expect(Self.presses(rig.inputs) == [Self.press("arrow_left", KeyMapping.control)])
        #expect(keyboard.control == .off)

        let before = rig.inputs.count
        keyboard.toggle(.control)
        view.setMarkedText("c", selectedRange: NSRange(location: 1, length: 0))
        view.unmarkText()
        #expect(
            Self.presses(rig.inputs(after: before)) == [KeyMapping.forText("c", mods: KeyMapping.control)],
            "a composition after the tap still gets the modifier"
        )
    }

    @Test("a terminal letting the keyboard go finishes its composition there, and keys go nowhere after")
    internal func lettingGo() {
        let first = Self.rig()
        let keyboard = first.view.keyboard
        let second = TerminalRig(clipboard: nil, keyboard: keyboard)
        first.view.setMarkedText("vi", selectedRange: NSRange(location: 2, length: 0))
        _ = first.view.resignFirstResponder()
        #expect(first.inputs == [.typed("vi")])
        #expect(keyboard.target == nil)
        keyboard.press("escape")
        keyboard.target = second.view
        keyboard.type("x")
        #expect(first.inputs == [.typed("vi")], "nothing reached the first after it let go")
        #expect(second.inputs == [.typed("x")])
    }

    @Test("a terminal the selection leaves finishes its composition there, then types nothing while it keeps the focus")
    internal func selectionLeft() {
        let rig = Self.rig()
        let view = rig.view
        let keyboard = view.keyboard
        view.setMarkedText("vi", selectedRange: NSRange(location: 2, length: 0))
        view.drawsFocused = false
        #expect(rig.inputs == [.typed("vi")])
        #expect(keyboard.target === view, "it keeps the focus until the next pane takes it")
        keyboard.toggle(.control)
        keyboard.type("x")
        keyboard.press("escape")
        view.insertText("y\n")
        view.deleteBackward()
        #expect(rig.inputs == [.typed("vi")], "nothing reached it after the selection left")
        #expect(keyboard.control == .once, "the sticky Ctrl waits for a key that goes somewhere")

        view.drawsFocused = true
        keyboard.type("c")
        #expect(Self.presses(rig.inputs) == [KeyMapping.forText("c", mods: KeyMapping.control)])
    }
}
