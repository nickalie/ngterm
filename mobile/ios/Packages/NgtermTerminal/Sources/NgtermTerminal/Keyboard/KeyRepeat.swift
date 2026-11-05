import Foundation

/// A held key's repeat: its action runs again after a moment, then steadily, until the key is let go (mobile/android's
/// extra keys: after 400 ms, every 50 ms).
@preconcurrency
@MainActor
internal final class KeyRepeat {
    /// How long a key is held before it repeats.
    internal static let delay: TimeInterval = 0.4
    /// How often it repeats then.
    internal static let interval: TimeInterval = 0.05

    private var timer: Timer?
    private var action: (@MainActor () -> Void)?

    /// A repeat with no key held.
    internal init() {
        // a key starts it
    }

    /// A key went down: `action` runs again after ``delay``, then every ``interval``.
    ///
    /// A key held before is let go.
    @preconcurrency
    internal func start(_ action: @escaping @MainActor () -> Void) {
        stop()
        self.action = action
        schedule(after: Self.delay)
    }

    /// The key was let go.
    internal func stop() {
        timer?.invalidate()
        timer = nil
        action = nil
    }

    // one repeat, as the timer runs it: the action, and the next one an interval later
    private func fire() {
        guard let action else {
            return
        }
        action()
        // the action may have let the key go
        if self.action != nil {
            schedule(after: Self.interval)
        }
    }

    private func schedule(after delay: TimeInterval) {
        timer?.invalidate()
        let made = Timer(timeInterval: delay, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.fire()
            }
        }
        RunLoop.main.add(made, forMode: .common)
        timer = made
    }

    isolated deinit {
        timer?.invalidate()
    }
}
