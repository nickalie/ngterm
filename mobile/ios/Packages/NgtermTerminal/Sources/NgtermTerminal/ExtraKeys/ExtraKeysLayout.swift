/// How the extra keys lay out, as Settings sets it (mobile/android's `ExtraKeysChoice`).
public enum ExtraKeysLayout: Sendable {
    /// Laid out for the width: two lines on a phone held upright, one once every key gets 44 points.
    case fit
    /// No bar: the soft keyboard alone.
    case hidden
    /// Two lines at every width: wider keys on a tablet or a phone on its side.
    case twoLines
}
