import UIKit

/// Says the cursor's line to VoiceOver as it changes, at most once per ``interval``: a line that changes faster is
/// said once the interval is up, as it stands then.
///
/// A line said once isn't said again until it changes, and a blank one isn't said.
@MainActor
internal final class CursorAnnouncer {
    /// The least time between two announcements, in seconds.
    internal static let interval: TimeInterval = 1

    private let now: @MainActor () -> TimeInterval
    private let post: @MainActor (String) -> Void
    private let later: @MainActor (TimeInterval, @escaping @MainActor () -> Void) -> Void
    private var lastSaid: TimeInterval = -.infinity
    private var lastSeen: String?
    private var waiting: String?
    private var isScheduled = false

    /// An announcer saying lines with `post`, reading the time from `now` and running what waits through `later`
    /// (after so many seconds).
    internal init(
        now: @escaping @MainActor () -> TimeInterval,
        post: @escaping @MainActor (String) -> Void,
        later: @escaping @MainActor (TimeInterval, @escaping @MainActor () -> Void) -> Void
    ) {
        self.now = now
        self.post = post
        self.later = later
    }

    /// An announcer saying lines to VoiceOver, while it runs, by the system's clock.
    internal convenience init() {
        self.init(now: { ProcessInfo.processInfo.systemUptime }, post: Self.announce, later: Self.after)
    }

    // VoiceOver hears it, while it runs
    private static func announce(_ line: String) {
        if UIAccessibility.isVoiceOverRunning {
            UIAccessibility.post(notification: .announcement, argument: line)
        }
    }

    private static func after(_ seconds: TimeInterval, _ action: @escaping @MainActor () -> Void) {
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(seconds))
            action()
        }
    }

    /// The cursor's line reads `line` now.
    internal func cursorLine(_ line: String) {
        guard line != lastSeen else {
            return
        }
        lastSeen = line
        guard !line.isEmpty else {
            waiting = nil
            return
        }
        let elapsed = now() - lastSaid
        if elapsed >= Self.interval {
            say(line)
            return
        }
        waiting = line
        if !isScheduled {
            isScheduled = true
            later(Self.interval - elapsed) { [weak self] in
                self?.sayWaiting()
            }
        }
    }

    /// Says the line that waited for the interval, if one still does.
    internal func sayWaiting() {
        isScheduled = false
        if let waiting {
            say(waiting)
        }
    }

    private func say(_ line: String) {
        waiting = nil
        lastSaid = now()
        post(line)
    }

    deinit {
        // a waiting line's task holds it weakly
    }
}
