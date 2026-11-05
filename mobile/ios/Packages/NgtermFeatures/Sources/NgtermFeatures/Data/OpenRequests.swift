import Observation

/// Where the app was asked to go from outside its screens, by a notification's tap (mobile/android's `OpenRequests`).
///
/// The ``pending`` target stays until a window has gone there and consumed it (``consume(_:)``). A later request
/// replaces one not yet taken.
@preconcurrency
@MainActor
@Observable
public final class OpenRequests {
    /// The target the app should go to, if any.
    public private(set) var pending: OpenTarget?

    /// No request yet.
    public init() {
        // nothing pending until a notification is tapped
    }

    /// Asks the app to go to `target`.
    public func request(_ target: OpenTarget) {
        pending = target
    }

    /// A window has gone to `target`.
    public func consume(_ target: OpenTarget) {
        if pending == target {
            pending = nil
        }
    }

    deinit {
        // nothing to release
    }
}
