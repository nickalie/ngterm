import UIKit

/// The grace on iOS: a background task (`beginBackgroundTask`) that runs out after ``defaultLimit``, or sooner when the
/// system takes its time back.
///
/// Run out by its own limit, it keeps the task a moment longer, so what its `expired` started (the core closing its
/// connections, an answer written first) gets to run; taken back by the system, it ends the task at once, as the system
/// asks.
@preconcurrency
@MainActor
public final class SystemGrace: BackgroundGrace {
    /// How long the grace lasts, well within the system's background time.
    public static let defaultLimit = Duration.seconds(limitSeconds)
    private static let limitSeconds = 20
    // how long the task is kept after the grace's own limit, for what `expired` started
    private static let settling = Duration.seconds(1)
    private static let taskName = "ngterm-grace"

    private let limit: Duration
    private var task = UIBackgroundTaskIdentifier.invalid
    private var timer: Task<Void, Never>?
    private var expired: (@MainActor () -> Void)?

    /// A grace of `limit`.
    public init(limit: Duration = SystemGrace.defaultLimit) {
        self.limit = limit
    }

    /// Begins the grace, ending one running.
    @preconcurrency
    public func begin(expired: @escaping @MainActor () -> Void) {
        end()
        self.expired = expired
        task = UIApplication.shared.beginBackgroundTask(withName: Self.taskName) { [weak self] in
            self?.runOut(isSettling: false)
        }
        timer = Task { [weak self, limit] in
            try? await Task.sleep(for: limit)
            guard !Task.isCancelled else {
                return
            }
            self?.runOut(isSettling: true)
        }
    }

    /// Ends the grace: its `expired` doesn't run, and the task ends.
    public func end() {
        timer?.cancel()
        timer = nil
        expired = nil
        finish(task)
        task = .invalid
    }

    private func runOut(isSettling: Bool) {
        timer?.cancel()
        timer = nil
        let run = expired
        expired = nil
        let ending = task
        task = .invalid
        run?()
        guard isSettling else {
            finish(ending)
            return
        }
        Task {
            try? await Task.sleep(for: Self.settling)
            finish(ending)
        }
    }

    private func finish(_ ending: UIBackgroundTaskIdentifier) {
        if ending != .invalid {
            UIApplication.shared.endBackgroundTask(ending)
        }
    }

    deinit {
        // nothing to release: a task still running ends with the grace's own limit
    }
}
