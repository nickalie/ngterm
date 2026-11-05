/// Where a sticky modifier of the extra keys stands: a tap arms it for the next key, a second tap locks it, a third
/// lets it go.
public enum Sticky: Equatable, Sendable {
    /// Held until tapped again.
    case locked
    /// Not held.
    case off
    /// Held for the next key, which lets it go.
    case once

    /// Where a tap moves it: off to once, once to locked, locked to off.
    public var next: Self {
        switch self {
        case .off:
            .once

        case .once:
            .locked

        case .locked:
            .off
        }
    }
}
