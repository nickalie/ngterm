import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// Waits for what happens on the main actor after an await: the core's events reach `Connections` through a stream,
/// and some calls run in tasks of their own.
@MainActor
internal enum Wait {
    // how often a condition is looked at, and for how long
    private static let stepMilliseconds = 2
    private static let limitSeconds = 10
    // how many turns the main actor gets once the events are in, for what they started
    private static let turns = 10

    /// Waits until `condition` holds, recording an issue if it doesn't within the limit.
    internal static func until(_ condition: () -> Bool) async {
        let deadline = ContinuousClock.now + .seconds(limitSeconds)
        while !condition() {
            guard ContinuousClock.now < deadline else {
                Issue.record("timed out waiting")
                return
            }
            try? await Task.sleep(for: .milliseconds(stepMilliseconds))
        }
    }

    /// Waits until `connections` has taken every event `core` has handed out so far, then lets whatever that started
    /// run a little.
    internal static func settled(_ connections: Connections, _ core: FakeMobileCore) async {
        await until { connections.handledEvents >= core.emitted.count }
        for _ in 0..<turns {
            await Task.yield()
        }
    }
}
