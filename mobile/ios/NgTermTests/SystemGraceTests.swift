import NgtermPlatform
import Testing

/// The grace on iOS, a background task the hosted app begins (an unhosted test bundle has no application to ask): it
/// runs out by its own limit, and an end or a new grace calls the one running off.
@MainActor
@Suite("the system's grace", .timeLimit(.minutes(1)))
internal struct SystemGraceTests {
    /// What a grace's `expired` did.
    @MainActor
    internal final class Runs {
        internal private(set) var times = 0

        internal func add() {
            times += 1
        }

        deinit {
            // nothing to release
        }
    }

    private static let limit = Duration.milliseconds(50)

    /// Begins `grace` and waits until it runs out; `then` runs as it does.
    private static func runOut(_ grace: SystemGrace, then: (@MainActor () -> Void)? = nil) async {
        await withCheckedContinuation { done in
            grace.begin {
                then?()
                done.resume()
            }
        }
    }

    @Test("the grace runs out by its limit, running its expired once, and an end after that changes nothing")
    internal func runsOutOnce() async {
        let grace = SystemGrace(limit: Self.limit)
        let runs = Runs()
        await Self.runOut(grace, then: runs.add)
        grace.end()
        #expect(runs.times == 1)
    }

    @Test("an ended grace never runs its expired, while one begun after it runs out on its own")
    internal func endedGraceNeverRunsOut() async {
        let ended = SystemGrace(limit: Self.limit)
        let runs = Runs()
        ended.begin(expired: runs.add)
        ended.end()
        await Self.runOut(SystemGrace(limit: Self.limit * 4))
        #expect(runs.times == 0)
    }

    @Test("beginning a grace again calls the running one off: only the new one's expired runs")
    internal func beginningAgainReplaces() async {
        let grace = SystemGrace(limit: Self.limit)
        let first = Runs()
        let second = Runs()
        grace.begin(expired: first.add)
        await Self.runOut(grace, then: second.add)
        #expect(first.times == 0)
        #expect(second.times == 1)
    }

    @Test("a grace runs out again each time it is begun anew")
    internal func beginsAnew() async {
        let grace = SystemGrace(limit: Self.limit)
        let runs = Runs()
        await Self.runOut(grace, then: runs.add)
        await Self.runOut(grace, then: runs.add)
        #expect(runs.times == 2)
    }
}
