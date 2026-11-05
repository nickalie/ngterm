import Testing

@testable import NgtermTerminal

/// The cursor's line said to VoiceOver as it changes, rate-limited.
@MainActor
@Suite("the cursor line's announcements")
internal struct CursorAnnouncerTests {
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
}
