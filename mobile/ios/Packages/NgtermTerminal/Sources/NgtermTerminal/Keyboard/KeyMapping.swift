import NgtermCore
import UIKit

/// UIKit's keys as the pane's (`PaneKey`, ngterm-term's `KeyInput`), as mobile/android's `KeyMapping` makes Android's:
/// the key's position by Ghostty's name, the modifiers as the pane's bits, the text the layout types before Ctrl and
/// Alt, the character the key types with no modifier (the Kitty protocol's key code), and the action.
///
/// A key's code is its position (its USB HID usage), whatever the layout types there; the characters come from UIKit.
/// What a key types plainly is the text system's (``route(_:)``), so dead keys and input methods work as they do in
/// any text view; special keys and chords go to the program as keys.
///
/// **Option** is Alt, unless the layout gives it a character of its own in ASCII (`@`, `|`, `[`, `~` on European
/// layouts), which it types: the US layout's Option characters (`∫`, `œ`) are no use in a terminal, which wants
/// Alt+B there, as Ghostty's `macos-option-as-alt` does for a US layout by default.
public enum KeyMapping {
    /// Shift's bit.
    public static let shift: UInt8 = 1
    /// Ctrl's bit.
    public static let control: UInt8 = 2
    /// Alt's (Option's) bit.
    public static let alt: UInt8 = 4
    /// Super's (Command's) bit.
    public static let `super`: UInt8 = 8
    /// Caps Lock's bit.
    public static let capsLock: UInt8 = 16

    private static let firstPrintable: UInt32 = 0x20
    private static let controls: ClosedRange<UInt32> = 0x7F...0x9F
    private static let lastASCII: UInt32 = 0x7E

    /// Ghostty's name of the key at the position `code`.
    ///
    /// A key Ghostty has no name for is `unidentified`.
    public static func physicalKey(_ code: UIKeyboardHIDUsage) -> String {
        names[code] ?? unidentified
    }

    /// The modifiers of `flags` as the pane's bits.
    ///
    /// Shift is 1, ctrl 2, alt 4, super 8, caps lock 16. UIKit doesn't say whether Num Lock is on.
    public static func mods(_ flags: UIKeyModifierFlags) -> UInt8 {
        var bits = flags.paneBits
        if flags.contains(.alphaShift) {
            bits |= capsLock
        }
        return bits
    }

    /// Where `stroke` goes: to the text system when it types plainly (no Ctrl or Command, and Option only for a
    /// character of the layout's own), to the program as a key when it is special or a chord.
    public static func route(_ stroke: KeyStroke) -> KeyRoute {
        let name = physicalKey(stroke.code)
        if modifierKeys.contains(name) {
            return .modifier
        }
        if special.contains(name) || isChord(stroke) {
            return .key
        }
        if name == unidentified, printable(stroke.characters).isEmpty {
            return .system
        }
        return .text
    }

    /// `stroke` as the pane's key, pressed.
    public static func key(_ stroke: KeyStroke) -> PaneKey {
        key(stroke, action: .press)
    }

    /// `stroke` as the pane's key, for `action`.
    public static func key(_ stroke: KeyStroke, action: PaneKeyAction) -> PaneKey {
        let name = physicalKey(stroke.code)
        let mods = mods(stroke.modifiers)
        let isSpecial = special.contains(name) || modifierKeys.contains(name)
        let typed = isSpecial ? "" : printable(typed(stroke))
        let own = isSpecial ? nil : optionCharacter(stroke)
        let text = own ?? typed
        let usedUp = own == nil ? shift : shift | alt
        return PaneKey(
            action: action,
            key: name,
            mods: mods,
            consumed: text.isEmpty ? 0 : mods & usedUp,
            composing: false,
            text: text,
            unshifted: isSpecial ? nil : unshifted(stroke, typed: typed)
        )
    }

    /// A press of the key that types `text` (one character) on a US keyboard, with `mods` held besides the shift the
    /// character needs: how a character the soft keyboard types takes the extra keys' sticky Ctrl and Alt. A character
    /// no US key types is an unidentified key with its text.
    public static func forText(_ text: String, mods: UInt8) -> PaneKey {
        let character = text.count == 1 ? text.first : nil
        let position = character.flatMap { usKeys[$0] }
        let shifted = position.map { $0.unshifted != character } ?? false
        let shiftBit = shifted ? shift : 0
        return PaneKey(
            action: .press,
            key: position?.name ?? unidentified,
            mods: mods | shiftBit,
            consumed: shiftBit,
            composing: false,
            text: text,
            unshifted: position.map { String($0.unshifted) }
        )
    }

    /// `key` for `action`: its repeat or its release.
    public static func key(_ key: PaneKey, as action: PaneKeyAction) -> PaneKey {
        var changed = key
        changed.action = action
        return changed
    }

    /// A press of the key Ghostty names `name` with `mods` held, typing nothing (Esc, Tab, the arrows, ...).
    public static func named(_ name: String, mods: UInt8) -> PaneKey {
        named(name, mods: mods, action: .press)
    }

    /// The key Ghostty names `name` with `mods` held, for `action`, typing nothing.
    public static func named(_ name: String, mods: UInt8, action: PaneKeyAction) -> PaneKey {
        PaneKey(action: action, key: name, mods: mods, consumed: 0, composing: false, text: "", unshifted: nil)
    }

    // whether Ctrl or Command is held, or Option for anything but a character of its own
    private static func isChord(_ stroke: KeyStroke) -> Bool {
        let held = stroke.modifiers
        if held.contains(.control) || held.contains(.command) {
            return true
        }
        return held.contains(.alternate) && optionCharacter(stroke) == nil
    }

    // what the key types with Shift but not Ctrl, Option or Command: UIKit's characters when none of those is held,
    // else the characters ignoring the modifiers, a letter made a capital under Shift where UIKit left it small
    private static func typed(_ stroke: KeyStroke) -> String {
        let held = stroke.modifiers
        guard held.contains(.control) || held.contains(.alternate) || held.contains(.command) else {
            return stroke.characters
        }
        let plain = stroke.charactersIgnoringModifiers
        return held.contains(.shift) ? plain.uppercased() : plain
    }

    // the character Option types of the layout's own: printable ASCII, other than the key's own character, with
    // neither Ctrl nor Command held
    private static func optionCharacter(_ stroke: KeyStroke) -> String? {
        let held = stroke.modifiers
        guard held.contains(.alternate), !held.contains(.control), !held.contains(.command) else {
            return nil
        }
        let characters = stroke.characters
        let scalars = characters.unicodeScalars
        let isASCII = scalars.count == 1 && scalars.allSatisfy { (firstPrintable...lastASCII).contains($0.value) }
        let ownCharacter = characters.lowercased() != stroke.charactersIgnoringModifiers.lowercased()
        return isASCII && ownCharacter ? characters : nil
    }

    // the character the key types with no modifier: a letter's small one; else the characters ignoring the modifiers
    // when Shift isn't held or UIKit left it out of them
    private static func unshifted(_ stroke: KeyStroke, typed: String) -> String? {
        let plain = printable(stroke.charactersIgnoringModifiers)
        if plain.count == 1, let letter = plain.first, letter.isLetter {
            return plain.lowercased()
        }
        let shiftKept = stroke.modifiers.contains(.shift) && plain == typed
        return plain.isEmpty || shiftKept ? nil : plain
    }

    // `text` when every character of it prints, else nothing: control characters and UIKit's names of special keys
    private static func printable(_ text: String) -> String {
        let prints = text.unicodeScalars.allSatisfy { scalar in
            scalar.value >= firstPrintable && !controls.contains(scalar.value)
        }
        return prints && !text.hasPrefix("UIKeyInput") ? text : ""
    }
}
