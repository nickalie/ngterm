import UIKit

/// A hardware key as UIKit reports it (`UIKey`): the key's position, the modifiers held, and what it types.
///
/// `UIKey` has no initializer of its own, so the key map takes this, and tests make one for a layout of their own.
public struct KeyStroke: Equatable {
    /// The key's position, by its USB HID usage: a layout's keys keep their US places whatever they type.
    public let code: UIKeyboardHIDUsage
    /// The modifiers held.
    public let modifiers: UIKeyModifierFlags
    /// What the key types with every modifier held applied, as the layout has it (Option's own characters included);
    /// a special key's is UIKit's name for it (`UIKeyInputUpArrow`).
    public let characters: String
    /// What the key types with no modifier applied but, maybe, Shift.
    public let charactersIgnoringModifiers: String

    /// The key at `code` with `modifiers` held, typing `characters` (`charactersIgnoringModifiers` without them).
    public init(
        code: UIKeyboardHIDUsage,
        modifiers: UIKeyModifierFlags,
        characters: String,
        charactersIgnoringModifiers: String
    ) {
        self.code = code
        self.modifiers = modifiers
        self.characters = characters
        self.charactersIgnoringModifiers = charactersIgnoringModifiers
    }

    /// The key UIKit reported.
    @preconcurrency
    @MainActor
    public init(_ key: UIKey) {
        self.init(
            code: key.keyCode,
            modifiers: key.modifierFlags,
            characters: key.characters,
            charactersIgnoringModifiers: key.charactersIgnoringModifiers
        )
    }
}
