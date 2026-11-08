import Foundation
import Testing

@testable import NgtermTerminal

/// The cursor's line said to VoiceOver as it changes, rate-limited.
@MainActor
@Suite("the cursor line's announcements")
internal struct CursorAnnouncerTests {
    /// A clock the test moves, what an announcer said, and the waits it scheduled, to run as their time comes.
    @MainActor
    private final class Waits {
        var time: TimeInterval = 100
        var said: [String] = []
        var actions: [@MainActor () -> Void] = []

        deinit {
            // nothing to release
        }
    }

    @Test("a changed line is said at once, the same line not again, and a blank one not at all")
    internal func changedLines() {
        let bench = AnnouncerBench()
        bench.announcer.cursorLine("$ ls")
        bench.announcer.cursorLine("$ ls")
        bench.time += 2
        bench.announcer.cursorLine("")
        bench.time += 2
        bench.announcer.cursorLine("$ make")
        #expect(bench.said == ["$ ls", "$ make"])
        #expect(bench.scheduled.isEmpty)
    }

    @Test("lines changing faster than once a second are said once the second is up, as they stand then")
    internal func rateLimited() {
        let bench = AnnouncerBench()
        bench.announcer.cursorLine("$ l")
        bench.time += 0.25
        bench.announcer.cursorLine("$ ls")
        bench.time += 0.25
        bench.announcer.cursorLine("$ ls -")
        #expect(bench.said == ["$ l"])
        #expect(bench.scheduled == [0.75], "one wait, for the rest of the second")
        bench.time += 0.5
        bench.announcer.sayWaiting()
        #expect(bench.said == ["$ l", "$ ls -"])
        bench.announcer.sayWaiting()
        #expect(bench.said.count == 2, "nothing waits any more")
    }

    @Test("a line that goes blank while it waits isn't said")
    internal func blankedWhileWaiting() {
        let bench = AnnouncerBench()
        bench.announcer.cursorLine("$ l")
        bench.time += 0.5
        bench.announcer.cursorLine("$ ls")
        bench.announcer.cursorLine("")
        bench.announcer.sayWaiting()
        #expect(bench.said == ["$ l"])
    }

    @Test("the wait it schedules says the line that waited, as it stands when the wait is up")
    internal func scheduledWait() throws {
        let waits = Waits()
        let announcer = CursorAnnouncer(
            now: { waits.time },
            post: { waits.said.append($0) },
            later: { _, action in waits.actions.append(action) }
        )
        announcer.cursorLine("$ l")
        waits.time += 0.25
        announcer.cursorLine("$ ls")
        announcer.cursorLine("$ ls -l")
        #expect(waits.actions.count == 1, "one wait for the lines in the second")
        waits.time += 0.75
        let wait = try #require(waits.actions.first)
        wait()
        #expect(waits.said == ["$ l", "$ ls -l"])
        waits.time += 0.25
        announcer.cursorLine("$ ls -la")
        #expect(waits.actions.count == 2, "a new wait once the last one ran")
    }
}
