/// A sticky modifier of the extra keys, with its bit in the pane's modifiers.
public enum StickyModifier: CaseIterable, Sendable {
    /// Alt.
    case alt
    /// Ctrl.
    case control

    /// Its bit in the pane's modifiers.
    public var bit: UInt8 {
        switch self {
        case .control:
            KeyMapping.control

        case .alt:
            KeyMapping.alt
        }
    }
}
