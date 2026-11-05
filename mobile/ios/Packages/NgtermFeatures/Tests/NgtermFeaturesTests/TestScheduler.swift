import NgtermPlatform

/// A ``RefreshScheduling`` that records whether a refresh is asked for.
@MainActor
internal final class TestScheduler: RefreshScheduling {
    /// Whether a refresh is asked for now.
    internal private(set) var isScheduled = false
    /// How many times one was asked for.
    internal private(set) var schedules = 0

    internal func schedule() {
        isScheduled = true
        schedules += 1
    }

    internal func cancel() {
        isScheduled = false
    }

    deinit {
        // nothing to release
    }
}
