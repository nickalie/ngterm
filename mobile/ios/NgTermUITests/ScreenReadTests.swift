import XCTest

/// The UI tests' waits (``ScreenRead``): what counts as one of their reads, and what tells them an element has gone.
///
/// On a loaded runner a read of the screen can take many seconds and then fail. A wait must count only the reads that
/// completed, and an element has gone only when a read completed and found no match.
internal final class ScreenReadTests: XCTestCase {
    // the error XCTest gives a read that matched nothing, and the one it gives a read of an app that does not run
    private static let uiTestingErrors = "com.apple.dt.xctest.ui-testing.error"
    private static let noMatches = 10_008
    private static let notRunning = 10_001
    // an app that is not installed, so no read of it completes
    private static let noApp = "com.nickalie.ngterm.no-such-app"

    // a wait a beat of 0.1 s apart, of an app that runs or that stopped
    @MainActor
    private static func wait(
        reads: Int,
        limit: TimeInterval,
        blindLimit: TimeInterval,
        appStopped: Bool = false
    ) -> ScreenRead.Wait {
        var wait = ScreenRead.Wait(reads: reads, beat: 0.1, limit: limit, blindLimit: blindLimit)
        wait.appStopped = { appStopped }
        return wait
    }

    @MainActor
    internal func testAReadThatFailedIsNotOneOfTheWaitsReads() {
        var calls = 0
        let polled = ScreenRead.Wait(reads: 3, beat: 0).poll {
            calls += 1
            return calls <= 2 ? .failed : .doesNotHold
        }
        XCTAssertEqual(polled, ScreenRead.Polled(held: false, reads: 3, failed: 2, seconds: polled.seconds))
        XCTAssertEqual(calls, 5)
    }

    @MainActor
    internal func testReadsThatAllFailEndTheWaitOnlyAtItsBlindLimit() {
        let polled = Self.wait(reads: 3, limit: 0.2, blindLimit: 1).poll { ScreenRead.Answer.failed }
        XCTAssertFalse(polled.held)
        XCTAssertEqual(polled.reads, 0)
        XCTAssertEqual(polled.answer, .failed)
        XCTAssertGreaterThanOrEqual(polled.seconds, 1)
    }

    // the CI read that failed for 205 s: the time limit of a wait does not end it before a read tells it anything
    @MainActor
    internal func testAWaitPastItsLimitHoldsAtTheFirstReadThatCompletes() {
        let start = Date.now
        let polled = Self.wait(reads: 3, limit: 0.5, blindLimit: 5).poll {
            Date.now.timeIntervalSince(start) >= 1 ? .holds : .failed
        }
        XCTAssertTrue(polled.held)
        XCTAssertEqual(polled.reads, 1)
        XCTAssertGreaterThanOrEqual(polled.seconds, 1)
    }

    // a first read past the limit that does not hold: the wait reads on, as its limit counts from that read
    @MainActor
    internal func testAWaitPastItsLimitReadsOnAfterAReadThatDoesNotHold() {
        let start = Date.now
        var completed = 0
        let polled = Self.wait(reads: 1_000, limit: 0.5, blindLimit: 60).poll {
            guard Date.now.timeIntervalSince(start) >= 1 else {
                return .failed
            }
            completed += 1
            return ScreenRead.Answer(completed == 2)
        }
        XCTAssertTrue(polled.held)
        XCTAssertEqual(polled.reads, 2)
    }

    @MainActor
    internal func testReadsOfAStoppedAppEndTheWaitAtItsLimit() {
        let polled = Self.wait(reads: 3, limit: 0.2, blindLimit: 60, appStopped: true)
            .poll { ScreenRead.Answer.failed }
        XCTAssertFalse(polled.held)
        XCTAssertEqual(polled.answer, .failed)
        XCTAssertGreaterThanOrEqual(polled.seconds, 0.2)
        XCTAssertLessThan(polled.seconds, 60)
    }

    @MainActor
    internal func testOnceAReadCompletedTheWaitEndsAtItsLimit() {
        var calls = 0
        let polled = ScreenRead.Wait(reads: 1_000, beat: 0.1, limit: 1, blindLimit: 60).poll {
            calls += 1
            return calls == 1 ? .doesNotHold : .failed
        }
        XCTAssertFalse(polled.held)
        XCTAssertEqual(polled.reads, 1)
        XCTAssertEqual(polled.answer, .doesNotHold)
        XCTAssertLessThan(polled.seconds, 60)
    }

    @MainActor
    internal func testAWaitReadsForItsTimeAtLeast() {
        let polled = ScreenRead.Wait(reads: 1, lasting: 1, beat: 0.1).poll { ScreenRead.Answer.doesNotHold }
        XCTAssertFalse(polled.held)
        XCTAssertGreaterThanOrEqual(polled.seconds, 1)
        XCTAssertGreaterThan(polled.reads, 1)
    }

    @MainActor
    internal func testAWaitEndsAtTheReadThatHolds() {
        var calls = 0
        let polled = ScreenRead.Wait(beat: 0).poll {
            calls += 1
            return ScreenRead.Answer(calls == 2)
        }
        XCTAssertTrue(polled.held)
        XCTAssertEqual(calls, 2)
    }

    @MainActor
    internal func testAReadNowIsTheFirstReadThatCompleted() {
        var calls = 0
        let answer = ScreenRead.now {
            calls += 1
            return calls <= 2 ? .failed : .doesNotHold
        }
        XCTAssertEqual(answer, .doesNotHold)
        XCTAssertEqual(calls, 3)
    }

    internal func testAWaitAnswersFromTheReadsThatCompleted() {
        XCTAssertEqual(ScreenRead.Polled(held: true, reads: 1, failed: 4, seconds: 0).answer, .holds)
        XCTAssertEqual(ScreenRead.Polled(held: false, reads: 1, failed: 4, seconds: 0).answer, .doesNotHold)
        XCTAssertEqual(ScreenRead.Polled(held: false, reads: 0, failed: 4, seconds: 0).answer, .failed)
    }

    internal func testOnlyAReadThatMatchedNothingReadsAbsent() {
        let noMatch = NSError(domain: Self.uiTestingErrors, code: Self.noMatches)
        XCTAssertEqual(ScreenRead.reading(after: noMatch), .absent)
        let appGone = NSError(domain: Self.uiTestingErrors, code: Self.notRunning)
        XCTAssertEqual(ScreenRead.reading(after: appGone), .failed)
        XCTAssertEqual(ScreenRead.reading(after: NSError(domain: NSCocoaErrorDomain, code: Self.noMatches)), .failed)
    }

    // on the app, a read finds what shows and reads absent what does not: the error XCTest gives is the one
    // `reading(after:)` takes for no match
    @MainActor
    internal func testAReadOfTheAppFindsWhatShowsAndWhatDoesNot() {
        let app = FakeApp.launch(withKey: true, for: self)
        FakeApp.showMachines(in: app)
        let add = app.buttons["add-machine"]
        XCTAssertEqual(ScreenRead.reading(add), .shown)
        XCTAssertEqual(ScreenRead.isGone(add), .doesNotHold)
        XCTAssertEqual(ScreenRead.shows(add, matching: NSPredicate(format: "identifier == 'add-machine'")), .holds)
        let missing = app.descendants(matching: .any)["no-such-element"].firstMatch
        XCTAssertEqual(ScreenRead.reading(missing), .absent)
        XCTAssertEqual(ScreenRead.isGone(missing), .holds)
        XCTAssertEqual(ScreenRead.shows(missing) { _ in true }, .doesNotHold)
        XCTAssertEqual(ScreenRead.reading(app.windows.element(boundBy: 99)), .absent)
        XCTAssertEqual(ScreenRead.isShown(add), .holds)
        XCTAssertEqual(ScreenRead.isShown(missing), .doesNotHold)
        XCTAssertEqual(ScreenRead.isShown(app.windows.element(boundBy: 0)), .holds)
        XCTAssertEqual(ScreenRead.isShown(app.windows.element(boundBy: 99)), .doesNotHold)
        // a query that matches more than one element shows, as `exists` tells, though a snapshot of it fails
        let many = app.descendants(matching: .any).element
        XCTAssertTrue(many.exists)
        XCTAssertThrowsError(try many.snapshot())
        XCTAssertEqual(ScreenRead.isShown(many), .holds)
        XCTAssertEqual(ScreenRead.isGone(many), .doesNotHold)
        FakeApp.expectGone(missing, "an element that never showed")
        FakeApp.expect(add, "an element that shows")
    }

    // a wait's own check of the app: false while the app runs, true once it stopped, and a blind wait of a stopped app
    // ends at its limit
    @MainActor
    internal func testTheWaitTellsWhetherTheAppRuns() {
        let app = FakeApp.launch(withKey: true, for: self)
        FakeApp.showMachines(in: app)
        XCTAssertFalse(ScreenRead.Wait().appStopped())
        app.terminate()
        XCTAssertTrue(ScreenRead.Wait().appStopped())
        let blindLimit: TimeInterval = 60
        let polled = ScreenRead.Wait(reads: 3, beat: 0.1, limit: 0.5, blindLimit: blindLimit).poll {
            ScreenRead.isShown(app.buttons["add-machine"])
        }
        XCTAssertEqual(polled.answer, .failed)
        XCTAssertLessThan(polled.seconds, blindLimit)
    }

    // a read that fails says that the element does not exist, as a read that found none does: only the read of the
    // snapshot tells them apart
    @MainActor
    internal func testAReadThatFailedTellsNothing() {
        let elsewhere = XCUIApplication(bundleIdentifier: Self.noApp).buttons["add-machine"]
        XCTAssertFalse(elsewhere.exists)
        XCTAssertEqual(ScreenRead.reading(elsewhere), .failed)
        XCTAssertEqual(ScreenRead.isGone(elsewhere), .failed)
        XCTAssertEqual(ScreenRead.isShown(elsewhere), .failed)
        XCTAssertEqual(ScreenRead.shows(elsewhere) { _ in true }, .failed)
        let polled = ScreenRead.Wait(reads: 1, beat: 0, blindLimit: 1).poll { ScreenRead.isGone(elsewhere) }
        XCTAssertEqual(polled.reads, 0)
        XCTAssertGreaterThan(polled.failed, 0)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
