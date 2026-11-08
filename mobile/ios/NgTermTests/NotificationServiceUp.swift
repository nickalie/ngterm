import Testing
import UserNotifications

/// Holds a suite until the system's notification service answers, so that a test's time limit counts the test's own
/// work and not the service's start.
///
/// A freshly booted simulator's notification service answers nothing until the system has started it, and the system
/// starts it in its own time: on CI's virtual Macs its first answer came 94 seconds after the first call on the phone
/// and more than 3 minutes after it on the tablet (SpringBoard's notification server still waiting on the background
/// task scheduler over 5 minutes after the boot, every app's calls held with it), and each call after it in
/// milliseconds. The suite waits for that first answer once, before its tests and outside their limits, and fails with
/// ``Down`` when it hasn't come within ``limit``, rather than the first test that happens to run absorbing the
/// service's start into its own limit.
internal struct NotificationServiceUp: SuiteTrait, TestScoping {
    /// The notification service didn't answer within the wait's limit.
    internal struct Down: Error, CustomStringConvertible {
        /// How long the wait was.
        internal var limit: Duration

        internal var description: String {
            "the system's notification service didn't answer within \(limit): the simulator hasn't finished starting it"
        }
    }

    // well past the slowest start seen (over 5 minutes from the tablet's boot), and well inside the CI job's limit
    private static let limitSeconds = 600
    /// How long the suite waits for the service's first answer.
    internal static let limit = Duration.seconds(limitSeconds)

    /// Waits until `answer` returns, or throws ``Down`` after `limit`; `answer` need not end on cancellation, as the
    /// system's calls don't.
    internal static func wait(
        for answer: @escaping @MainActor @Sendable () async -> Void,
        limit: Duration
    ) async throws {
        try Task.checkCancellation()
        let (answers, continuation) = AsyncStream<Void>.makeStream()
        Task { @MainActor in
            await answer()
            continuation.yield()
            continuation.finish()
        }
        let deadline = Task {
            try? await Task.sleep(for: limit)
            continuation.finish()
        }
        defer {
            deadline.cancel()
        }
        var iterator = answers.makeAsyncIterator()
        let answered = await iterator.next() != nil
        try Task.checkCancellation()
        guard answered else {
            throw Down(limit: limit)
        }
    }

    // a call every app's notifications make, answered once the service is up
    @MainActor
    private static func ask() async {
        _ = await UNUserNotificationCenter.current().notificationSettings()
    }

    internal func provideScope(
        for _: Test,
        testCase _: Test.Case?,
        performing function: @Sendable () async throws -> Void
    ) async throws {
        try await Self.wait(for: Self.ask, limit: Self.limit)
        try await function()
    }
}
