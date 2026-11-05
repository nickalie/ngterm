import NgtermCore
import NgtermTerminal
import Testing
import UIKit

/// UIKit's keys turned into the pane's (`PaneKey`, ngterm-term's `KeyInput` over the binding): the key's position by
/// Ghostty's name, the modifier bits, the text before Ctrl and Alt, the unshifted character and the action; which keys
/// the text system types and which go to the program as keys; a character typed with sticky modifiers; and the app's
/// chords (mobile/android's `KeyMappingTest`, for iOS's keys and the Mac's chords).
@MainActor
@Suite("the key map")
internal struct KeyMappingTests {
    // keys by position, and the names Ghostty gives them
    private static let named: [(code: UIKeyboardHIDUsage, name: String)] = [
        (UIKeyboardHIDUsage.keyboardA, "key_a"),
        (.keyboardZ, "key_z"),
        (.keyboard0, "digit_0"),
        (.keyboard1, "digit_1"),
        (.keyboard9, "digit_9"),
        (.keyboardReturnOrEnter, "enter"),
        (.keyboardDeleteOrBackspace, "backspace"),
        (.keyboardDeleteForward, "delete"),
        (.keyboardEscape, "escape"),
        (.keyboardTab, "tab"),
        (.keyboardSpacebar, "space"),
        (.keyboardUpArrow, "arrow_up"),
        (.keyboardLeftArrow, "arrow_left"),
        (.keyboardHome, "home"),
        (.keyboardEnd, "end"),
        (.keyboardPageUp, "page_up"),
        (.keyboardPageDown, "page_down"),
        (.keyboardF1, "f1"),
        (.keyboardF12, "f12"),
        (.keyboardF24, "f24"),
        (.keypad5, "numpad_5"),
        (.keypad0, "numpad_0"),
        (.keypadEnter, "numpad_enter"),
        (.keyboardGraveAccentAndTilde, "backquote"),
        (.keyboardQuote, "quote"),
        (.keyboardNonUSBackslash, "intl_backslash"),
        (.keyboardInternational3, "intl_yen"),
        (.keyboardLeftControl, "control_left"),
        (.keyboardRightGUI, "meta_right"),
        (.keyboardLeftAlt, "alt_left"),
        (.keyboardVolumeUp, "unidentified"),
    ]

    private static func stroke(
        _ code: UIKeyboardHIDUsage,
        _ characters: String,
        _ ignoring: String? = nil
    ) -> KeyStroke {
        stroke(code, [], characters, ignoring)
    }

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

    private static func key(
        _ name: String,
        _ text: String = "",
        mods: UInt8 = 0,
        consumed: UInt8 = 0,
        unshifted: String? = nil,
        action: PaneKeyAction = .press
    ) -> PaneKey {
        PaneKey(
            action: action,
            key: name,
            mods: mods,
            consumed: consumed,
            composing: false,
            text: text,
            unshifted: unshifted
        )
    }

    @Test("keys are named by their position as Ghostty names them")
    internal func names() {
        for (code, name) in Self.named {
            #expect(KeyMapping.physicalKey(code) == name, "\(code.rawValue)")
        }
    }

    @Test("the modifiers held are the pane's bits")
    internal func modifierBits() {
        #expect(KeyMapping.mods([]) == 0)
        #expect(KeyMapping.mods(.shift) == KeyMapping.shift)
        #expect(KeyMapping.mods(.control) == KeyMapping.control)
        #expect(KeyMapping.mods(.alternate) == KeyMapping.alt)
        #expect(KeyMapping.mods(.command) == KeyMapping.super)
        #expect(KeyMapping.mods(.alphaShift) == KeyMapping.capsLock)
        #expect(KeyMapping.mods([.control, .shift]) == KeyMapping.control | KeyMapping.shift)
        #expect(KeyMapping.mods(.numericPad) == 0, "a numeric pad's key says where it is, not that Num Lock is on")
    }

    @Test("a letter types itself, and Shift is used up by the capital")
    internal func letters() {
        #expect(KeyMapping.key(Self.stroke(.keyboardA, "a")) == Self.key("key_a", "a", unshifted: "a"))
        let shift = KeyMapping.shift
        #expect(
            KeyMapping.key(Self.stroke(.keyboardA, .shift, "A", "a"))
                == Self.key("key_a", "A", mods: shift, consumed: shift, unshifted: "a")
        )
        #expect(
            KeyMapping.key(Self.stroke(.keyboard1, .shift, "!", "1"))
                == Self.key("digit_1", "!", mods: shift, consumed: shift, unshifted: "1")
        )
        // UIKit may keep Shift in the characters ignoring the modifiers: the unshifted letter is still its own
        #expect(
            KeyMapping.key(Self.stroke(.keyboardA, .shift, "A", "A"))
                == Self.key("key_a", "A", mods: shift, consumed: shift, unshifted: "a")
        )
    }

    @Test("Ctrl and Alt leave the text as the layout types it")
    internal func ctrlAndAlt() {
        #expect(
            KeyMapping.key(Self.stroke(.keyboardC, .control, "\u{3}", "c"))
                == Self.key("key_c", "c", mods: KeyMapping.control, unshifted: "c")
        )
        #expect(
            KeyMapping.key(Self.stroke(.keyboardX, .alternate, "≈", "x"))
                == Self.key("key_x", "x", mods: KeyMapping.alt, unshifted: "x")
        )
        let ctrlShift = KeyMapping.control | KeyMapping.shift
        #expect(
            KeyMapping.key(Self.stroke(.keyboardC, [.control, .shift], "\u{3}", "c"))
                == Self.key("key_c", "C", mods: ctrlShift, consumed: KeyMapping.shift, unshifted: "c")
        )
        #expect(
            KeyMapping.key(Self.stroke(.keyboardK, .command, "k"))
                == Self.key("key_k", "k", mods: KeyMapping.super, unshifted: "k")
        )
    }

    @Test("Shift in a chord types the shifted character, which UIKit leaves out")
    internal func shiftInChords() {
        let (shift, ctrlShift) = (KeyMapping.shift, KeyMapping.control | KeyMapping.shift)
        // UIKit's Ctrl+Shift+- is U+001F and '-', as Ctrl+- is: the US layout's '_' is Ghostty's Ctrl+_ (0x1F)
        #expect(
            KeyMapping.key(Self.stroke(.keyboardHyphen, [.control, .shift], "\u{1F}", "-"))
                == Self.key("minus", "_", mods: ctrlShift, consumed: shift, unshifted: "-")
        )
        #expect(
            KeyMapping.key(Self.stroke(.keyboard6, [.control, .shift], "6"))
                == Self.key("digit_6", "^", mods: ctrlShift, consumed: shift, unshifted: "6")
        )
        #expect(
            KeyMapping.key(Self.stroke(.keyboardSlash, [.control, .shift], "/"))
                == Self.key("slash", "?", mods: ctrlShift, consumed: shift, unshifted: "/")
        )
        #expect(
            KeyMapping.key(Self.stroke(.keyboard1, [.command, .shift], "1"))
                == Self.key("digit_1", "!", mods: KeyMapping.super | shift, consumed: shift, unshifted: "1")
        )
        // Option+Shift+- types '—' of the layout's own: Alt and '_'
        #expect(
            KeyMapping.key(Self.stroke(.keyboardHyphen, [.alternate, .shift], "\u{2014}", "-"))
                == Self.key("minus", "_", mods: KeyMapping.alt | shift, consumed: shift, unshifted: "-")
        )
        // characters UIKit gave with Shift are the layout's own
        #expect(
            KeyMapping.key(Self.stroke(.keyboard2, [.command, .shift], "\"", "2"))
                == Self.key("digit_2", "\"", mods: KeyMapping.super | shift, consumed: shift, unshifted: "2")
        )
        // a key typing another than the US character at its place keeps it: German '-' at US '/'
        #expect(
            KeyMapping.key(Self.stroke(.keyboardSlash, [.control, .shift], "\u{1F}", "-"))
                == Self.key("slash", "-", mods: ctrlShift, consumed: shift)
        )
    }

    @Test("special keys type no text, whatever UIKit names them")
    internal func specialKeys() {
        #expect(KeyMapping.key(Self.stroke(.keyboardReturnOrEnter, "\r")) == Self.key("enter"))
        #expect(KeyMapping.key(Self.stroke(.keyboardTab, "\t")) == Self.key("tab"))
        #expect(KeyMapping.key(Self.stroke(.keyboardDeleteOrBackspace, "\u{8}")) == Self.key("backspace"))
        #expect(KeyMapping.key(Self.stroke(.keyboardEscape, UIKeyCommand.inputEscape)) == Self.key("escape"))
        #expect(KeyMapping.key(Self.stroke(.keyboardUpArrow, UIKeyCommand.inputUpArrow)) == Self.key("arrow_up"))
        #expect(KeyMapping.key(Self.stroke(.keyboardF5, "UIKeyInputF5")) == Self.key("f5"))
        #expect(
            KeyMapping.key(Self.stroke(.keyboardLeftArrow, [.control, .shift], UIKeyCommand.inputLeftArrow))
                == Self.key("arrow_left", mods: KeyMapping.control | KeyMapping.shift)
        )
    }

    @Test("a held key repeats and a lifted one is released")
    internal func actions() {
        let stroke = Self.stroke(.keyboardJ, "j")
        #expect(KeyMapping.key(stroke, action: .repeat) == Self.key("key_j", "j", unshifted: "j", action: .repeat))
        #expect(KeyMapping.key(stroke, action: .release) == Self.key("key_j", "j", unshifted: "j", action: .release))
    }

    @Test("a non-US layout types its own characters at US positions")
    internal func layouts() {
        // German: Z and Y swap, ß sits at US '-'; Dvorak: '/' at US '['
        #expect(KeyMapping.key(Self.stroke(.keyboardZ, "y")) == Self.key("key_z", "y", unshifted: "y"))
        #expect(KeyMapping.key(Self.stroke(.keyboardHyphen, "ß")) == Self.key("minus", "ß", unshifted: "ß"))
        #expect(KeyMapping.key(Self.stroke(.keyboardOpenBracket, "/")) == Self.key("bracket_left", "/", unshifted: "/"))
    }

    @Test("Option types the layout's own ASCII character and uses up Alt; any other is Alt and the key")
    internal func option() {
        let alt = KeyMapping.alt
        // the German Mac layout's Option+L and Option+7
        #expect(
            KeyMapping.key(Self.stroke(.keyboardL, .alternate, "@", "l"))
                == Self.key("key_l", "@", mods: alt, consumed: alt, unshifted: "l")
        )
        #expect(
            KeyMapping.key(Self.stroke(.keyboard7, .alternate, "|", "7"))
                == Self.key("digit_7", "|", mods: alt, consumed: alt, unshifted: "7")
        )
        // the US layout's Option+B types ∫ and the German one's Option+Q «, which a terminal has no use for: Alt+B and
        // Alt+Q for the program, as Ghostty's macos-option-as-alt has it on a US layout
        #expect(
            KeyMapping.key(Self.stroke(.keyboardB, .alternate, "∫", "b"))
                == Self.key("key_b", "b", mods: alt, unshifted: "b")
        )
        #expect(
            KeyMapping.key(Self.stroke(.keyboardQ, .alternate, "«", "q"))
                == Self.key("key_q", "q", mods: alt, unshifted: "q")
        )
        // a dead key under Option (the US layout's Option+E) types nothing of its own
        #expect(
            KeyMapping.key(Self.stroke(.keyboardE, .alternate, "", "e"))
                == Self.key("key_e", "e", mods: alt, unshifted: "e")
        )
        // with Ctrl held too, it is the chord
        #expect(
            KeyMapping.key(Self.stroke(.keyboardL, [.alternate, .control], "@", "l"))
                == Self.key("key_l", "l", mods: alt | KeyMapping.control, unshifted: "l")
        )
    }

    @Test("what types plainly is the text system's; special keys and chords are the program's keys")
    internal func routes() {
        #expect(KeyMapping.route(Self.stroke(.keyboardA, "a")) == .text)
        #expect(KeyMapping.route(Self.stroke(.keyboardA, .shift, "A", "a")) == .text)
        #expect(KeyMapping.route(Self.stroke(.keyboardA, .alphaShift, "A", "a")) == .text)
        #expect(KeyMapping.route(Self.stroke(.keyboardSpacebar, " ")) == .text)
        #expect(KeyMapping.route(Self.stroke(.keypad5, .numericPad, "5")) == .text)
        #expect(KeyMapping.route(Self.stroke(.keyboardEqualSign, "", "")) == .text, "a dead key composes")
        #expect(KeyMapping.route(Self.stroke(.keyboardL, .alternate, "@", "l")) == .text, "Option's own character")
        #expect(KeyMapping.route(Self.stroke(.keyboardB, .alternate, "∫", "b")) == .key)
        #expect(KeyMapping.route(Self.stroke(.keyboardC, .control, "\u{3}", "c")) == .key)
        #expect(KeyMapping.route(Self.stroke(.keyboardK, .command, "k")) == .key)
        #expect(KeyMapping.route(Self.stroke(.keyboardReturnOrEnter, "\r")) == .key)
        #expect(KeyMapping.route(Self.stroke(.keyboardDeleteOrBackspace, "\u{8}")) == .key)
        #expect(KeyMapping.route(Self.stroke(.keyboardEscape, UIKeyCommand.inputEscape)) == .key)
        #expect(KeyMapping.route(Self.stroke(.keyboardTab, .shift, "\t")) == .key)
        #expect(KeyMapping.route(Self.stroke(.keypadEnter, .numericPad, "\r")) == .key)
        #expect(KeyMapping.route(Self.stroke(.keyboardLeftShift, .shift, "")) == .modifier)
        #expect(KeyMapping.route(Self.stroke(.keyboardCapsLock, .alphaShift, "")) == .modifier)
        #expect(KeyMapping.route(Self.stroke(.keyboardVolumeUp, "")) == .system)
    }

    @Test("a character with sticky modifiers is the key that types it on a US layout")
    internal func forText() {
        let ctrl = KeyMapping.control
        let alt = KeyMapping.alt
        let shift = KeyMapping.shift
        #expect(KeyMapping.forText("c", mods: ctrl) == Self.key("key_c", "c", mods: ctrl, unshifted: "c"))
        #expect(
            KeyMapping.forText("D", mods: alt)
                == Self.key("key_d", "D", mods: alt | shift, consumed: shift, unshifted: "d")
        )
        #expect(
            KeyMapping.forText("|", mods: 0)
                == Self.key("backslash", "|", mods: shift, consumed: shift, unshifted: "\\")
        )
        #expect(KeyMapping.forText("/", mods: 0) == Self.key("slash", "/", unshifted: "/"))
        #expect(KeyMapping.forText(" ", mods: ctrl) == Self.key("space", " ", mods: ctrl, unshifted: " "))
        #expect(KeyMapping.forText("é", mods: alt) == Self.key("unidentified", "é", mods: alt))
    }

    @Test("a named key types nothing")
    internal func named() {
        #expect(KeyMapping.named("escape", mods: 0) == Self.key("escape"))
        #expect(
            KeyMapping.named("arrow_up", mods: KeyMapping.alt, action: .release)
                == Self.key("arrow_up", mods: KeyMapping.alt, action: .release)
        )
    }

    @Test("the app's chords are the Mac's, as upstream binds them")
    internal func chords() {
        let commandOption: UIKeyModifierFlags = [.command, .alternate]
        let chords = TerminalShortcut.listed.map { "\($0.input) \($0.modifiers.rawValue)" }
        #expect(
            chords == [
                "+ \(UIKeyModifierFlags.command.rawValue)",
                "- \(UIKeyModifierFlags.command.rawValue)",
                "0 \(UIKeyModifierFlags.command.rawValue)",
                "\(UIKeyCommand.inputRightArrow) \(commandOption.rawValue)",
                "\(UIKeyCommand.inputLeftArrow) \(commandOption.rawValue)",
                "\(UIKeyCommand.inputDownArrow) \(commandOption.rawValue)",
                "\(UIKeyCommand.inputUpArrow) \(commandOption.rawValue)",
            ]
        )
        #expect(TerminalShortcut.listed.allSatisfy { !$0.title.isEmpty })
    }

    @Test("a stroke is its chord's whatever the layout puts the character under, and only with its modifiers")
    internal func matching() {
        // ⌘+ is ⌘⇧= on a US layout, and ⌘= does as well; German has '+' a key of its own
        #expect(TerminalShortcut.matching(Self.stroke(.keyboardEqualSign, [.command, .shift], "+", "=")) == .bigger)
        #expect(TerminalShortcut.matching(Self.stroke(.keyboardEqualSign, .command, "=")) == .bigger)
        #expect(TerminalShortcut.matching(Self.stroke(.keyboardCloseBracket, .command, "+")) == .bigger)
        #expect(TerminalShortcut.matching(Self.stroke(.keyboardHyphen, .command, "-")) == .smaller)
        #expect(TerminalShortcut.matching(Self.stroke(.keyboard0, .command, "0")) == .resetSize)
        #expect(
            TerminalShortcut.matching(
                Self.stroke(.keyboardRightArrow, [.command, .alternate], UIKeyCommand.inputRightArrow)
            ) == .nextPane
        )
        #expect(
            TerminalShortcut.matching(
                Self.stroke(.keyboardUpArrow, [.command, .alternate], UIKeyCommand.inputUpArrow)
            ) == .previousSession
        )
        // Caps Lock doesn't count; anything else held, or nothing, is the program's
        #expect(TerminalShortcut.matching(Self.stroke(.keyboardHyphen, [.command, .alphaShift], "-")) == .smaller)
        #expect(TerminalShortcut.matching(Self.stroke(.keyboardHyphen, "-")) == nil)
        #expect(TerminalShortcut.matching(Self.stroke(.keyboardHyphen, .control, "-")) == nil)
        #expect(TerminalShortcut.matching(Self.stroke(.keyboardHyphen, [.command, .control], "-")) == nil)
        #expect(
            TerminalShortcut.matching(Self.stroke(.keyboardRightArrow, .command, UIKeyCommand.inputRightArrow)) == nil
        )
        #expect(TerminalShortcut.matching(Self.stroke(.keyboardC, .command, "c")) == nil)
    }
}
