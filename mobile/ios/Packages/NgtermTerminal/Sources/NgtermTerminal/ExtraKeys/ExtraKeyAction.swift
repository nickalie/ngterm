/// What an extra key does.
public enum ExtraKeyAction: Equatable, Sendable {
    /// Arms, locks or lets go of a sticky modifier.
    case hold(StickyModifier)
    /// Presses the key Ghostty names `name`; `repeats` while held.
    case press(String, repeats: Bool)
    /// Opens or closes the soft keyboard, keeping the focus.
    case softKeyboard
    /// Types `text`, as the key that types it on a US keyboard.
    case type(String)
}
