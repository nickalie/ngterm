import NgtermCore
import NgtermCoreFakes
import NgtermTerminal
import Testing
import UIKit

/// A hardware keyboard's keys against the fake pane, which records the input: what goes to the program as keys, what
/// the text system types, and the app's chords (mobile/android's `TerminalKeyboardTest`, its hardware half).
@MainActor
@Suite("a hardware keyboard")
internal struct HardwareKeysTests {
    private static func stroke(
        _ code: UIKeyboardHIDUsage,
        _ modifiers: UIKeyModifierFlags,
        _ characters: String,
        _ ignoring: String? = nil
    ) -> KeyStroke {
        KeyStroke(
            code: code,
            modifiers: modifiers,
            characters: characters,
            charactersIgnoringModifiers: ignoring ?? characters
        )
    }

    // the keys a pane took, in order
    private static func keys(_ inputs: [FakeInput]) -> [PaneKey] {
        inputs.compactMap { input in
            if case .key(let key) = input {
                return key
            }
            return nil
        }
    }

    // a key pressed and let go: whether the view took each
    private static func strike(_ stroke: KeyStroke, on view: TerminalView) -> (down: Bool, up: Bool) {
        (view.keyDown(stroke), view.keyUp(stroke))
    }

    @Test("what types plainly is the text system's; a chord and a special key are the program's, pressed and let go")
    internal func keys() {
        let rig = TerminalRig(clipboard: nil)
        let view = rig.view
        #expect(Self.strike(Self.stroke(.keyboardA, [], "a"), on: view) == (false, false))
        #expect(Self.strike(Self.stroke(.keyboardA, .shift, "A", "a"), on: view) == (false, false))
        #expect(rig.inputs.isEmpty, "the text system types it, through insertText")
        let ctrlC = Self.stroke(.keyboardC, .control, "\u{3}", "c")
        #expect(Self.strike(ctrlC, on: view) == (true, true))
        let enter = Self.stroke(.keyboardReturnOrEnter, [], "\r")
        #expect(Self.strike(enter, on: view) == (true, true))
        let pressed = KeyMapping.key(ctrlC)
        #expect(
            Self.keys(rig.inputs) == [
                pressed, KeyMapping.key(pressed, as: .release), KeyMapping.named("enter", mods: 0),
                KeyMapping.named("enter", mods: 0, action: .release),
            ]
        )
    }

    @Test("a modifier key goes to the program and the system both")
    internal func modifiers() {
        let rig = TerminalRig(clipboard: nil)
        let shift = Self.stroke(.keyboardLeftShift, .shift, "")
        #expect(Self.strike(shift, on: rig.view) == (false, false))
        #expect(Self.keys(rig.inputs).map(\.key) == ["shift_left", "shift_left"])
        #expect(Self.keys(rig.inputs).map(\.action) == [.press, .release])
    }

    @Test("while an input method composes, every key is its own")
    internal func composing() {
        let rig = TerminalRig(clipboard: nil)
        rig.view.setMarkedText("か", selectedRange: NSRange(location: 1, length: 0))
        #expect(Self.strike(Self.stroke(.keyboardReturnOrEnter, [], "\r"), on: rig.view) == (false, false))
        #expect(Self.strike(Self.stroke(.keyboardLeftArrow, [], "UIKeyInputLeftArrow"), on: rig.view) == (false, false))
        #expect(rig.inputs.isEmpty)
    }

    @Test("a sticky modifier joins the next special key, which keeps it to its release")
    internal func sticky() {
        let rig = TerminalRig(clipboard: nil)
        rig.view.keyboard.toggle(.control)
        let left = Self.stroke(.keyboardLeftArrow, [], "UIKeyInputLeftArrow")
        _ = Self.strike(left, on: rig.view)
        let ctrlLeft = KeyMapping.named("arrow_left", mods: KeyMapping.control)
        #expect(Self.keys(rig.inputs) == [ctrlLeft, KeyMapping.key(ctrlLeft, as: .release)])
        #expect(rig.view.keyboard.control == .off)
    }

    @Test("a held special key repeats until let go")
    internal func repeats() async throws {
        let rig = TerminalRig(clipboard: nil)
        let down = Self.stroke(.keyboardDownArrow, [], "UIKeyInputDownArrow")
        #expect(rig.view.keyDown(down))
        try await Task.sleep(for: .milliseconds(700))
        #expect(rig.view.keyUp(down))
        let actions = Self.keys(rig.inputs).map(\.action)
        #expect(actions.first == .press)
        #expect(actions.last == .release)
        #expect(actions.filter { $0 == .repeat }.count >= 2, "\(actions)")
        let count = rig.inputs.count
        try await Task.sleep(for: .milliseconds(200))
        #expect(rig.inputs.count == count)
    }

    @Test("the text size's chords change the text, and never reach the program")
    internal func textSize() {
        let rig = TerminalRig(clipboard: nil)
        let view = rig.view
        let size = view.textSize
        #expect(Self.strike(Self.stroke(.keyboardEqualSign, [.command, .shift], "+", "="), on: view) == (true, true))
        #expect(Self.strike(Self.stroke(.keyboardEqualSign, .command, "="), on: view) == (true, true))
        #expect(view.textSize == size + 2)
        _ = Self.strike(Self.stroke(.keyboardHyphen, .command, "-"), on: view)
        #expect(view.textSize == size + 1)
        _ = Self.strike(Self.stroke(.keyboard0, .command, "0"), on: view)
        #expect(view.textSize == size)
        #expect(rig.inputs.isEmpty)
        view.textSize = TerminalPointer.textSizes.upperBound
        view.run(.bigger)
        #expect(view.textSize == TerminalPointer.textSizes.upperBound, "within Settings' sizes")
    }

    @Test("the moves run the session view's actions; a move with none is the program's chord")
    internal func moves() {
        let rig = TerminalRig(clipboard: nil)
        let view = rig.view
        let moves = Locked<[String]>([])
        view.keyboard.shortcuts = TerminalShortcuts(
            nextPane: { moves.value.append("next pane") },
            previousPane: { moves.value.append("previous pane") },
            nextSession: { moves.value.append("next session") }
        )
        let held: UIKeyModifierFlags = [.command, .alternate]
        _ = Self.strike(Self.stroke(.keyboardRightArrow, held, "UIKeyInputRightArrow"), on: view)
        _ = Self.strike(Self.stroke(.keyboardLeftArrow, held, "UIKeyInputLeftArrow"), on: view)
        _ = Self.strike(Self.stroke(.keyboardDownArrow, held, "UIKeyInputDownArrow"), on: view)
        _ = Self.strike(Self.stroke(.keyboardUpArrow, held, "UIKeyInputUpArrow"), on: view)
        #expect(moves.value == ["next pane", "previous pane", "next session"])
        let pressed = Self.keys(rig.inputs).filter { $0.action == .press }
        #expect(pressed == [KeyMapping.named("arrow_up", mods: KeyMapping.super | KeyMapping.alt)])
    }

    @Test("it lists the chords it has actions for while ⌘ is held")
    internal func keyCommands() {
        let view = TerminalRig(clipboard: nil).view
        let titles = { Set((view.keyCommands ?? []).map(\.title)) }
        #expect(titles() == ["Bigger Text", "Smaller Text", "Default Text Size"])
        view.keyboard.shortcuts = TerminalShortcuts(
            nextPane: {
                // a move the session view would make
            },
            previousSession: {
                // a move the session view would make
            }
        )
        #expect(titles() == ["Bigger Text", "Smaller Text", "Default Text Size", "Next Pane", "Previous Session"])
        let hidden = (view.keyCommands ?? []).filter { $0.attributes.contains(.hidden) }
        #expect(hidden.map(\.input) == ["="], "⌘= beside ⌘+, unlisted")
    }
}
