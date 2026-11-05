import UIKit

extension KeyMapping {
    /// The name of a key no table here names.
    internal static let unidentified = "unidentified"

    /// The keys that type no character: their names, the program's keys whatever is held.
    internal static let special: Set<String> = {
        var found: Set = [
            "escape", "enter", "tab", "backspace", "delete", "insert", "home", "end", "page_up", "page_down",
            "arrow_up", "arrow_down", "arrow_left", "arrow_right", "numpad_enter", "print_screen", "scroll_lock",
            "pause", "help", "context_menu", "num_lock", "copy", "cut", "paste", "power",
        ]
        for function in 1...functionKeys {
            found.insert("f\(function)")
        }
        return found
    }()

    /// The modifier keys, by name.
    internal static let modifierKeys: Set = [
        "shift_left", "shift_right", "control_left", "control_right", "alt_left", "alt_right", "meta_left",
        "meta_right", "caps_lock",
    ]

    /// Every key by its position (its USB HID usage), as Ghostty names it.
    internal static let names: [UIKeyboardHIDUsage: String] = {
        var table = Self.fixedNames
        let letters: [UIKeyboardHIDUsage] = [
            .keyboardA, .keyboardB, .keyboardC, .keyboardD, .keyboardE, .keyboardF, .keyboardG, .keyboardH,
            .keyboardI, .keyboardJ, .keyboardK, .keyboardL, .keyboardM, .keyboardN, .keyboardO, .keyboardP,
            .keyboardQ, .keyboardR, .keyboardS, .keyboardT, .keyboardU, .keyboardV, .keyboardW, .keyboardX,
            .keyboardY, .keyboardZ,
        ]
        for (code, letter) in zip(letters, "abcdefghijklmnopqrstuvwxyz") {
            table[code] = "key_\(letter)"
        }
        let digits: [UIKeyboardHIDUsage] = [
            .keyboard0, .keyboard1, .keyboard2, .keyboard3, .keyboard4, .keyboard5, .keyboard6, .keyboard7,
            .keyboard8, .keyboard9,
        ]
        let pad: [UIKeyboardHIDUsage] = [
            .keypad0, .keypad1, .keypad2, .keypad3, .keypad4, .keypad5, .keypad6, .keypad7, .keypad8, .keypad9,
        ]
        for (index, (digit, keypad)) in zip(digits, pad).enumerated() {
            table[digit] = "digit_\(index)"
            table[keypad] = "numpad_\(index)"
        }
        let functions: [UIKeyboardHIDUsage] = [
            .keyboardF1, .keyboardF2, .keyboardF3, .keyboardF4, .keyboardF5, .keyboardF6, .keyboardF7, .keyboardF8,
            .keyboardF9, .keyboardF10, .keyboardF11, .keyboardF12, .keyboardF13, .keyboardF14, .keyboardF15,
            .keyboardF16, .keyboardF17, .keyboardF18, .keyboardF19, .keyboardF20, .keyboardF21, .keyboardF22,
            .keyboardF23, .keyboardF24,
        ]
        for (index, code) in functions.enumerated() {
            table[code] = "f\(index + 1)"
        }
        return table
    }()

    /// Every character a US keyboard types, with the key that types it and what that key types unshifted.
    internal static let usKeys: [Character: (name: String, unshifted: Character)] = {
        var keys: [Character: (name: String, unshifted: Character)] = [:]
        for letter in "abcdefghijklmnopqrstuvwxyz" {
            let key = (name: "key_\(letter)", unshifted: letter)
            keys[letter] = key
            keys[Character(letter.uppercased())] = key
        }
        let unshifted = Array("0123456789`-=[]\\;',./ ")
        let shifted = Array(")!@#$%^&*(~_+{}|:\"<>? ")
        let digits = "0123456789".map { "digit_\($0)" }
        let others = [
            "backquote", "minus", "equal", "bracket_left", "bracket_right", "backslash", "semicolon", "quote",
            "comma", "period", "slash", "space",
        ]
        for (index, name) in (digits + others).enumerated() {
            let key = (name: name, unshifted: unshifted[index])
            keys[unshifted[index]] = key
            keys[shifted[index]] = key
        }
        return keys
    }()

    private static let functionKeys = 24

    // the keys past the letters, digits, keypad digits and F1-F24
    private static let fixedNames: [UIKeyboardHIDUsage: String] = [
        .keyboardReturnOrEnter: "enter",
        .keyboardEscape: "escape",
        .keyboardDeleteOrBackspace: "backspace",
        .keyboardTab: "tab",
        .keyboardSpacebar: "space",
        .keyboardHyphen: "minus",
        .keyboardEqualSign: "equal",
        .keyboardOpenBracket: "bracket_left",
        .keyboardCloseBracket: "bracket_right",
        .keyboardBackslash: "backslash",
        // an ISO keyboard's key by Return, where the US one has backslash: the W3C code is Backslash for both
        .keyboardNonUSPound: "backslash",
        .keyboardSemicolon: "semicolon",
        .keyboardQuote: "quote",
        .keyboardGraveAccentAndTilde: "backquote",
        .keyboardComma: "comma",
        .keyboardPeriod: "period",
        .keyboardSlash: "slash",
        .keyboardCapsLock: "caps_lock",
        .keyboardPrintScreen: "print_screen",
        .keyboardScrollLock: "scroll_lock",
        .keyboardPause: "pause",
        .keyboardInsert: "insert",
        .keyboardHome: "home",
        .keyboardPageUp: "page_up",
        .keyboardDeleteForward: "delete",
        .keyboardEnd: "end",
        .keyboardPageDown: "page_down",
        .keyboardRightArrow: "arrow_right",
        .keyboardLeftArrow: "arrow_left",
        .keyboardDownArrow: "arrow_down",
        .keyboardUpArrow: "arrow_up",
        .keypadNumLock: "num_lock",
        .keypadSlash: "numpad_divide",
        .keypadAsterisk: "numpad_multiply",
        .keypadHyphen: "numpad_subtract",
        .keypadPlus: "numpad_add",
        .keypadEnter: "numpad_enter",
        .keypadPeriod: "numpad_decimal",
        .keypadEqualSign: "numpad_equal",
        .keypadComma: "numpad_comma",
        .keyboardNonUSBackslash: "intl_backslash",
        .keyboardApplication: "context_menu",
        .keyboardPower: "power",
        .keyboardHelp: "help",
        .keyboardCut: "cut",
        .keyboardCopy: "copy",
        .keyboardPaste: "paste",
        .keyboardInternational1: "intl_ro",
        .keyboardInternational2: "kana_mode",
        .keyboardInternational3: "intl_yen",
        .keyboardInternational4: "convert",
        .keyboardInternational5: "non_convert",
        .keyboardLeftControl: "control_left",
        .keyboardLeftShift: "shift_left",
        .keyboardLeftAlt: "alt_left",
        .keyboardLeftGUI: "meta_left",
        .keyboardRightControl: "control_right",
        .keyboardRightShift: "shift_right",
        .keyboardRightAlt: "alt_right",
        .keyboardRightGUI: "meta_right",
    ]
}
