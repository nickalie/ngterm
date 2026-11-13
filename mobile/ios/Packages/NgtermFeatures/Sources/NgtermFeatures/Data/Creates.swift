import Observation

/// The session create under way on this device, whichever screen began it (mobile/android's `Creates`): one at a
/// time, so two landings never race to open their session.
@preconcurrency
@MainActor
@Observable
public final class Creates {
    /// The create under way, or `nil`.
    public private(set) var underWay: Create?

    /// No create under way.
    public init() {
        // nothing under way yet
    }

    /// Begins `create`; false, beginning nothing, while another is under way.
    public func begin(_ create: Create) -> Bool {
        guard underWay == nil else {
            return false
        }
        underWay = create
        return true
    }

    /// Ends `create`, when it is the one under way.
    public func end(_ create: Create) {
        if underWay == create {
            underWay = nil
        }
    }

    deinit {
        // nothing to release
    }
}
