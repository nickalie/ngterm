import BackgroundTasks
import Foundation

/// The app's Background App Refresh (`BGAppRefreshTask`): registered at launch, asked for while sessions are attached,
/// and run by the system when it sees fit, usually in a process started afresh for it.
///
/// The work runs on the main actor until it ends or the system takes its time back, which cancels it; either way the
/// task is reported done then. The identifier is one the app's `Info.plist` permits
/// (`BGTaskSchedulerPermittedIdentifiers`), with `UIBackgroundModes` holding `fetch`.
@preconcurrency
@MainActor
public final class BackgroundRefresh: RefreshScheduling {
    /// The refresh's identifier, as the app's `Info.plist` permits it.
    public static let identifier = "com.nickalie.ngterm.ios.refresh"
    // how long after it is asked for a refresh may start at the earliest: the system decides when within it
    private static let interval: TimeInterval = 900

    /// The refresh, by the system's scheduler.
    public init() {
        // nothing to set up: register(_:) runs at launch
    }

    // runs `work` for `task`, cancelled when the system takes its time back
    private static func run(_ task: BGTask, _ work: @escaping @MainActor () async -> Void) {
        let job = Task { @MainActor in
            await work()
        }
        task.expirationHandler = {
            job.cancel()
        }
        Task { @MainActor in
            await job.value
            task.setTaskCompleted(success: !job.isCancelled)
        }
    }

    /// Registers `work` as the refresh; call it once, before the app finishes launching.
    ///
    /// - Returns: false when the system refused it (the identifier isn't permitted).
    @discardableResult
    @preconcurrency
    public func register(_ work: @escaping @MainActor () async -> Void) -> Bool {
        BGTaskScheduler.shared.register(forTaskWithIdentifier: Self.identifier, using: .main) { task in
            MainActor.assumeIsolated {
                Self.run(task, work)
            }
        }
    }

    /// Asks the system for a refresh some time from now, replacing one asked for before.
    public func schedule() {
        let request = BGAppRefreshTaskRequest(identifier: Self.identifier)
        request.earliestBeginDate = Date(timeIntervalSinceNow: Self.interval)
        // a device with Background App Refresh off refuses it: notifications then come only while the app runs
        try? BGTaskScheduler.shared.submit(request)
    }

    /// Calls off the refresh asked for, if any.
    public func cancel() {
        BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.identifier)
    }

    deinit {
        // nothing to release: the scheduler keeps what was asked for
    }
}
