/// Why a machine with attached sessions was lost for good (mobile/android's `LostReason`).
public enum LostReason: Hashable, Sendable {
    /// The machine offered a key other than the pinned one.
    case hostKeyChanged
    /// The core gave up, for this reason (its words).
    case unreachable(String)
}
