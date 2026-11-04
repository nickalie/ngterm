import Synchronization

/// A count any thread may add to, for the tests' callbacks.
internal final class Counter: Sendable {
    private let value = Mutex(0)

    /// The count so far.
    internal var count: Int {
        value.withLock { $0 }
    }

    /// Adds one.
    internal func add() {
        value.withLock { $0 += 1 }
    }

    deinit {
        // nothing to release
    }
}
