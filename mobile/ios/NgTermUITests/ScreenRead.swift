import XCTest

/// Reads of the app's screen for the UI tests' waits: a read completes and tells what the screen shows, or it fails and
/// tells nothing.
///
/// On a loaded runner one read can take 9-30 s, and a read that fails says that an element does not exist, as a read
/// that found none does (`exists` is false for both). So a wait counts only the reads that completed, and an element
/// has gone only when a read completed and found no match.
internal enum ScreenRead {
    /// What one read told about a condition.
    internal enum Answer: Equatable {
        /// The read completed, and the condition does not hold.
        case doesNotHold
        /// The read did not complete.
        case failed
        /// The read completed, and the condition holds.
        case holds

        internal init(_ holds: Bool) {
            self = holds ? .holds : .doesNotHold
        }
    }

    /// What one read found of an element.
    internal enum Reading: Equatable {
        /// The read completed and found no such element.
        case absent
        /// The read did not complete.
        case failed
        /// The element shows.
        case shown
    }

    /// What a wait came to.
    internal struct Polled: Equatable {
        /// Whether the condition held.
        internal let held: Bool
        /// The reads that completed, and those that did not.
        internal let reads: Int
        internal let failed: Int
        internal let seconds: TimeInterval

        /// What the wait's reads told: `.failed` when it held not and no read completed.
        internal var answer: Answer {
            if held {
                return .holds
            }
            return reads > 0 ? .doesNotHold : .failed
        }
    }

    /// A wait that reads a condition again, a beat apart, until it holds.
    ///
    /// It gives up after `reads` reads that completed and `lasting` seconds both, or after `limit` (or `lasting`, when
    /// that is longer) once a read completed. The count of reads is the bound, not a short time alone: on a loaded
    /// runner one read can take as long as a whole timed wait. The limit stops a wait whose reads are all that slow.
    /// A wait whose reads all failed knows nothing of the screen yet, so it gives up only at `blindLimit`.
    internal struct Wait {
        internal var reads = ScreenRead.reads
        internal var lasting: TimeInterval = 0
        internal var beat = ScreenRead.beat
        internal var limit = ScreenRead.limit
        internal var blindLimit = ScreenRead.blindLimit

        /// Reads `done` until it holds, and tells whether it did.
        ///
        /// A read that failed is not one of the `reads`.
        @MainActor
        @discardableResult
        internal func poll(until done: () -> Answer) -> Polled {
            let start = Date.now
            var count = 0
            var failed = 0
            while true {
                switch done() {
                case .holds:
                    let seconds = Date.now.timeIntervalSince(start)
                    return Polled(held: true, reads: count + 1, failed: failed, seconds: seconds)

                case .doesNotHold:
                    count += 1

                case .failed:
                    failed += 1
                }
                let seconds = Date.now.timeIntervalSince(start)
                let end = max(count > 0 ? limit : blindLimit, lasting)
                if (count >= reads && seconds >= lasting) || seconds >= end {
                    return Polled(held: false, reads: count, failed: failed, seconds: seconds)
                }
                if beat > 0 {
                    _ = XCTWaiter.wait(for: [XCTestExpectation(description: "a beat")], timeout: beat)
                }
            }
        }
    }

    /// How many reads that complete show that a step (a tap, a press) did not take, before it is done again: a step
    /// done again when it took (a switch pressed twice) undoes it.
    internal static let stepReads = 6
    /// How often a wait reads the screen, and the fewest reads it gives up after.
    internal static let beat: TimeInterval = 0.5
    internal static let reads = 60
    /// How long a wait reads the screen, at most, when its reads are slow: a failed wait must end well inside the CI
    /// job's time limit, its retry included.
    internal static let limit: TimeInterval = 180
    /// How long a wait reads the screen, at most, when none of its reads completed.
    ///
    /// On a runner at load 880 on 3 CPUs the app answered no read for 205 s (each read: 3 tries of 30 s), and the next
    /// read found the element. A test still ends inside its time allowance (UI.xctestplan).
    internal static let blindLimit: TimeInterval = 600
    // the error XCTest gives for a read of an element that completed and matched nothing
    private static let uiTestingErrors = "com.apple.dt.xctest.ui-testing.error"
    private static let noMatches = 10_008

    /// The wait after a step: `stepReads` reads, for `lasting` seconds at least.
    internal static func step(lasting: TimeInterval) -> Wait {
        Wait(reads: stepReads, lasting: lasting)
    }

    /// Waits until `done` holds, and fails the test when it doesn't.
    ///
    /// The failure says `failure`, with the app's hierarchy and a screenshot. The wait reads for `lasting` seconds at
    /// least.
    @MainActor
    internal static func expect(_ failure: String, lasting: TimeInterval, until done: () -> Answer) {
        let polled = Wait(lasting: lasting).poll(until: done)
        guard !polled.held else {
            return
        }
        FakeApp.attachScreen(failure)
        XCTFail("\(failure) after \(polled.reads) reads (\(polled.failed) failed) in \(Int(polled.seconds)) s")
    }

    /// Waits until `done` holds, as the expect of an answer does, each read taken as complete.
    ///
    /// Only for a condition whose read always completes (a file, the clipboard). A read of the screen fails at times,
    /// and `exists` is false then: read an element with ``isShown(_:)`` or ``isGone(_:)``.
    @MainActor
    internal static func expect(_ failure: String, lasting: TimeInterval, until done: () -> Bool) {
        expect(failure, lasting: lasting) { Answer(done()) }
    }

    /// Whether `done` holds, from the next read that completes: a read that fails is done again, up to the limit.
    ///
    /// Before a step is done again (a tap, a press), as the step can take after the last read of its wait: do the step
    /// again only on `.doesNotHold`. It is `.failed` when no read completed until the limit, and the state is then
    /// not known.
    @MainActor
    internal static func now(_ done: () -> Answer) -> Answer {
        Wait(reads: 1).poll(until: done).answer
    }

    /// Reads the screen once for `element`, from a whole snapshot of it.
    ///
    /// The read is of the first match: a query that matches more than one element shows, as `exists` tells.
    @MainActor
    internal static func reading(_ element: XCUIElement) -> Reading {
        do {
            _ = try element.firstMatch.snapshot()
            return .shown
        } catch {
            return reading(after: error)
        }
    }

    /// What a read of an element that threw `error` found: no such element, or nothing, when the read failed.
    internal static func reading(after error: any Error) -> Reading {
        let nsError = error as NSError
        return nsError.domain == uiTestingErrors && nsError.code == noMatches ? .absent : .failed
    }

    /// Whether `element` shows, from one read of the screen.
    @MainActor
    internal static func isShown(_ element: XCUIElement) -> Answer {
        switch reading(element) {
        case .absent:
            .doesNotHold

        case .failed:
            .failed

        case .shown:
            .holds
        }
    }

    /// Whether `element` has gone, from one read of the screen.
    @MainActor
    internal static func isGone(_ element: XCUIElement) -> Answer {
        switch reading(element) {
        case .absent:
            .holds

        case .failed:
            .failed

        case .shown:
            .doesNotHold
        }
    }

    /// Whether `element` shows and `holds` for it, from one read of the screen.
    @MainActor
    internal static func shows(_ element: XCUIElement, where holds: (any XCUIElementSnapshot) -> Bool) -> Answer {
        do {
            return Answer(holds(try element.snapshot()))
        } catch {
            return reading(after: error) == .absent ? .doesNotHold : .failed
        }
    }

    /// Whether `element` shows and matches `predicate` (as a query's predicate reads an element), from one read of the
    /// screen.
    @MainActor
    internal static func shows(_ element: XCUIElement, matching predicate: NSPredicate) -> Answer {
        shows(element) { predicate.evaluate(with: $0) }
    }
}
