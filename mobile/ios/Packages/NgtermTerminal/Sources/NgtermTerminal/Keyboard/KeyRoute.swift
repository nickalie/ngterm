/// Where a hardware key's press goes (``KeyMapping/route(_:)``).
public enum KeyRoute: Equatable, Sendable {
    /// The program's, as a key: a special key, or a chord.
    case key
    /// A modifier key: the program hears of it, and the system keeps it too.
    case modifier
    /// The system's: a key with no name and no text (the volume).
    case system
    /// The text system's: it types what the key types, through dead keys and input methods, as the soft keyboard does.
    case text
}
