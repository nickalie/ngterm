import Foundation
import Synchronization

/// A clock the test sets: what ``now`` reads is what it was last set to.
internal final class TestClock: Sendable {
    // where it starts: any fixed time will do
    private static let startSeconds: TimeInterval = 1_000_000

    private let current = Mutex(Date(timeIntervalSince1970: TestClock.startSeconds))

    /// The time it reads.
    internal var now: Date {
        get { current.withLock { $0 } }
        set { current.withLock { $0 = newValue } }
    }

    /// Reads the clock: what `Connections` counts a reconnect down by.
    internal var read: @Sendable () -> Date {
        { [self] in now }
    }

    deinit {
        // nothing to release
    }
}
