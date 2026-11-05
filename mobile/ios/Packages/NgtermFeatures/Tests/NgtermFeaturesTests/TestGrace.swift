import NgtermPlatform

/// A ``BackgroundGrace`` the test runs out by hand (``runOut()``), as the time running out would.
@MainActor
internal final class TestGrace: BackgroundGrace {
    private var expired: (@MainActor () -> Void)?

    /// Whether a grace is running.
    internal var isRunning: Bool {
        expired != nil
    }

    internal func begin(expired: @escaping @MainActor () -> Void) {
        self.expired = expired
    }

    internal func end() {
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
