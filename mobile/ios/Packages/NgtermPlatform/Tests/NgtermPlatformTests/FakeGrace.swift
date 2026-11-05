import NgtermPlatform

/// A ``BackgroundGrace`` the test runs out by hand (``runOut()``), as the time running out would.
@MainActor
internal final class FakeGrace: BackgroundGrace {
    /// How many graces began.
    internal private(set) var begun = 0
    /// How many ended before they ran out.
    internal private(set) var ended = 0
    private var expired: (@MainActor () -> Void)?

    /// Whether a grace is running.
    internal var isRunning: Bool {
        expired != nil
    }

    internal func begin(expired: @escaping @MainActor () -> Void) {
        begun += 1
        self.expired = expired
    }

    internal func end() {
        if expired != nil {
            ended += 1
        }
        expired = nil
    }

    /// The running grace runs out.
    internal func runOut() {
        let run = expired
        expired = nil
        run?()
    }

    deinit {
        // nothing to release
    }
}
