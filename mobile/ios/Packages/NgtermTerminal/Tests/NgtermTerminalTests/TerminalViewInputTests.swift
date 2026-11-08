import CoreText
import NgtermCoreFakes
import NgtermTerminal
import Testing
import UIKit

/// The view's input at its edges against the fake pane: presses with no key, keys held as it lets the keyboard go,
/// its chords as key commands, the extra keys as VoiceOver and a touch work them, and its font's faces.
@MainActor
@Suite("the terminal view's input")
internal struct TerminalViewInputTests {
    // a rig whose view is its keyboard's target, as the first responder would be
    private static func rig() -> TerminalRig {
        let rig = TerminalRig(clipboard: nil)
        rig.view.keyboard.target = rig.view
        return rig
    }

    // the extra key `key` of `rig`'s bar, laid out
    private static func button(_ key: ExtraKey, of rig: TerminalRig) throws -> UIControl {
        let bar = rig.view.keyboard.bar
        bar.frame = CGRect(x: 0, y: 0, width: 390, height: ExtraKeysBar.keyHeight * 2)
        bar.layoutIfNeeded()
        let found = Self.views(in: bar).first { $0.accessibilityIdentifier == key.identifier }
        return try #require(found as? UIControl)
    }

    // `view` and every view under it
    private static func views(in view: UIView) -> [UIView] {
        [view] + view.subviews.flatMap(views)
    }

    // a key pressed and let go
    private static func struck(_ name: String) -> [FakeInput] {
        [.key(KeyMapping.named(name, mods: 0)), .key(KeyMapping.named(name, mods: 0, action: .release))]
    }

    @Test("presses with no key on them (a remote's, a game controller's) go on to the system, not the program")
    internal func keylessPresses() {
        let rig = Self.rig()
        let presses: Set<UIPress> = [UIPress()]
        rig.view.pressesBegan(presses, with: nil)
        rig.view.pressesEnded(presses, with: nil)
        rig.view.pressesCancelled(presses, with: nil)
        #expect(rig.inputs.isEmpty)
    }

    @Test("a key held as the terminal lets the keyboard go is let go to the program, and the keys go nowhere after")
    internal func heldAsKeyboardGoes() {
        let rig = Self.rig()
        let view = rig.view
        let down = KeyStroke(
            code: .keyboardDownArrow,
            modifiers: [],
            characters: "UIKeyInputDownArrow",
            charactersIgnoringModifiers: "UIKeyInputDownArrow"
        )
        #expect(view.keyDown(down))
        _ = view.resignFirstResponder()
        #expect(rig.inputs == Self.struck("arrow_down"))
        #expect(!view.keyUp(down), "let go already")
        #expect(view.keyboard.target == nil)
        view.keyboard.press("escape")
        #expect(rig.inputs.count == 2)
    }

    @Test("the chords listed while ⌘ is held run as the key commands UIKit sends")
    internal func keyCommands() throws {
        let view = Self.rig().view
        let commands = view.keyCommands ?? []
        let run = { (title: String) in
            let command = try #require(commands.first { $0.title == title })
            let action = try #require(command.action)
            _ = view.perform(action, with: command)
        }
        try run("Bigger Text")
        #expect(view.textSize == TerminalRig.textSize + 1)
        try run("Bigger Text")
        try run("Smaller Text")
        #expect(view.textSize == TerminalRig.textSize + 1)
        try run("Default Text Size")
        #expect(view.textSize == TerminalRig.textSize)
    }

    @Test("hiding the soft keyboard while it is closed changes nothing")
    internal func hideClosedKeyboard() {
        let view = Self.rig().view
        view.hideSoftKeyboard()
        #expect(!view.softKeyboardWanted)
        #expect(view.inputView != nil, "still no keys")
    }

    @Test("new faces remake the font, none being the system's monospace; the same faces again keep it")
    internal func faces() {
        let rig = Self.rig()
        let view = rig.view
        let bundled = view.font
        let same = view.faces
        view.faces = same
        #expect(view.font === bundled)
        view.faces = nil
        view.layoutIfNeeded()
        let system = TerminalFont(size: TerminalRig.textSize, faces: nil)
        #expect(view.font !== bundled)
        #expect(view.font.cellWidth == system.cellWidth)
        #expect(view.font.cellHeight == system.cellHeight)
        #expect(rig.pane.sizes.last?.cols == UInt16((view.bounds.width / system.cellWidth).rounded(.down)))
    }

    @Test("the system's monospace has its bold, italic and bold italic faces")
    internal func systemFaces() {
        let font = TerminalFont(size: TerminalRig.textSize, faces: nil)
        let traits = { (attributes: TerminalRun.Attributes) in CTFontGetSymbolicTraits(font.face(for: attributes)) }
        #expect(!traits([]).contains(.traitBold) && !traits([]).contains(.traitItalic))
        #expect(traits(.bold).contains(.traitBold))
        #expect(traits(.italic).contains(.traitItalic))
        #expect(traits([.bold, .italic]).contains([.traitBold, .traitItalic]))
    }

    @Test("VoiceOver's value is the screen's text, whatever is set on it")
    internal func accessibilityValue() {
        let view = Self.rig().view
        view.accessibilityValue = "something else"
        #expect(view.accessibilityValue == "hello world\nsecond line\n\nthe end")
    }

    @Test("VoiceOver's activation of an extra key presses it and lets it go")
    internal func voiceOverActivation() throws {
        let rig = Self.rig()
        let escape = try Self.button(.escape, of: rig)
        #expect(escape.accessibilityActivate())
        #expect(rig.inputs == Self.struck("escape"))
    }

    @Test("the keyboard key closes the soft keyboard it opened, the focus staying")
    internal func keyboardKeyCloses() {
        let rig = Self.rig()
        let bar = rig.view.keyboard.bar
        bar.press(.keyboard)
        #expect(rig.view.softKeyboardWanted)
        bar.press(.keyboard)
        #expect(!rig.view.softKeyboardWanted)
        #expect(rig.view.keyboard.target === rig.view)
        #expect(rig.inputs.isEmpty)
    }
}
