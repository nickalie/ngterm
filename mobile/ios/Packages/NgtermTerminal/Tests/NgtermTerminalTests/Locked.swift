import Synchronization

/// A value a fake's closure and its test share, behind a lock.
internal final class Locked<Value: Sendable>: Sendable {
    private let mutex: Mutex<Value>

    /// The value now.
    internal var value: Value {
        get { mutex.withLock { $0 } }
        set { mutex.withLock { $0 = newValue } }
    }

    /// A lock around `value`.
    internal init(_ value: Value) {
        mutex = Mutex(value)
    }

    deinit {
        // nothing to release
    }
}
