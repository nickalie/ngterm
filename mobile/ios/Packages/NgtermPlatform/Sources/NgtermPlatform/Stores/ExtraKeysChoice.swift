/// The extra-keys bar over the keyboard.
public enum ExtraKeysChoice: StoredChoice {
    /// Laid out for the width: two lines on a phone held upright, one once each key has room.
    case fit
    /// No bar: the keyboard alone.
    case hidden
    /// Two lines at every width.
    case twoLines
}
