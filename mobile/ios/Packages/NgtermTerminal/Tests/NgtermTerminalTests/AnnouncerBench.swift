import Foundation

@testable import NgtermTerminal

/// A ``CursorAnnouncer`` on a clock the test moves, recording what it says and the waits it schedules.
@MainActor
internal final class AnnouncerBench {
    /// The clock, in seconds.
    internal var time: TimeInterval = 100
    /// What it said, in order.
    internal private(set) var said: [String] = []
    /// The waits it scheduled, in seconds.
    internal private(set) var scheduled: [TimeInterval] = []
    /// The announcer.
    internal lazy var announcer = CursorAnnouncer(
        now: { [weak self] in self?.time ?? 0 },
        post: { [weak self] line in self?.said.append(line) },
        later: { [weak self] seconds, _ in self?.scheduled.append(seconds) }
    )

    deinit {
        // nothing to release
    }
}
