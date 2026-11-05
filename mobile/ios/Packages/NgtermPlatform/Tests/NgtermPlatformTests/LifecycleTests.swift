import NgtermPlatform
import Testing

/// The app's life across its scenes, on a grace run out by hand and scenes the test makes (mobile/android's
/// `IdleKeeper` and `Foreground` cases): a grace, then a suspend; a suspend at once with nothing attached; none while a
/// machine waits for a sign-in; a resume back in the foreground; and one window of two leaving changes nothing.
@MainActor
@Suite("the app's lifecycle")
internal struct LifecycleTests {
    /// What the app holds, and the suspends and resumes made, for the lifecycle to read and write.
    @MainActor
    internal final class Calls {
        internal var holds = LifecycleHold.sessions
        internal var log: [String] = []

        deinit {
            // nothing to release
        }
    }

    private let grace: FakeGrace
    // scenes, as the test makes them: only their identity counts (a type's will do)
    private let window = ObjectIdentifier(Int.self)
    private let other = ObjectIdentifier(String.self)
    private let calls: Calls
    private let lifecycle: Lifecycle

    internal init() {
        let recorder = Calls()
        let fake = FakeGrace()
        calls = recorder
        grace = fake
        lifecycle = Lifecycle(grace: fake) {
            recorder.holds
        } suspend: {
            recorder.log.append("suspend")
        } resume: {
            recorder.log.append("resume")
        }
    }

    private func enter(_ scene: ObjectIdentifier) {
        lifecycle.sceneEntered(scene)
    }

    private func leave(_ scene: ObjectIdentifier) {
        lifecycle.sceneLeft(scene)
    }

    @Test("with sessions attached, leaving keeps a grace, and its end suspends")
    internal func graceThenSuspend() {
        enter(window)
        leave(window)
        #expect(grace.isRunning)
        #expect(calls.log.isEmpty)
        #expect(!lifecycle.isSuspended)
        grace.runOut()
        #expect(calls.log == ["suspend"])
        #expect(lifecycle.isSuspended)
        #expect(!lifecycle.isInForeground)
    }

    @Test("with nothing attached, leaving suspends at once")
    internal func nothingAttached() {
        calls.holds = .nothing
        enter(window)
        leave(window)
        #expect(calls.log == ["suspend"])
        #expect(grace.begun == 0)
    }

    @Test("no suspend while a machine waits for a sign-in, until it ends")
    internal func signIn() {
        calls.holds = .signIn
        enter(window)
        leave(window)
        #expect(calls.log.isEmpty)
        #expect(!grace.isRunning)
        // the sign-in ends with sessions attached: the grace begins only now
        calls.holds = .sessions
        lifecycle.holdsChanged()
        #expect(grace.isRunning)
        // a sign-in asked for again in the grace keeps the connections when it runs out
        calls.holds = .signIn
        grace.runOut()
        #expect(calls.log.isEmpty)
        calls.holds = .nothing
        lifecycle.holdsChanged()
        #expect(calls.log == ["suspend"])
    }

    @Test("back in the foreground resumes what a suspend let go, and ends a grace with nothing to resume")
    internal func resume() {
        enter(window)
        leave(window)
        enter(window)
        #expect(grace.ended == 1)
        #expect(calls.log.isEmpty, "back within the grace: nothing was let go")
        leave(window)
        grace.runOut()
        enter(window)
        #expect(calls.log == ["suspend", "resume"])
        #expect(!lifecycle.isSuspended)
        #expect(lifecycle.isInForeground)
    }

    @Test("one window of two going to the background suspends nothing")
    internal func oneWindowOfTwo() {
        enter(window)
        enter(other)
        leave(other)
        #expect(grace.begun == 0)
        #expect(calls.log.isEmpty)
        #expect(lifecycle.isInForeground)
        leave(window)
        #expect(grace.isRunning)
    }

    @Test("a launch in the background (a refresh) leaves the core alone")
    internal func backgroundLaunch() {
        calls.holds = .nothing
        lifecycle.holdsChanged()
        leave(window)
        #expect(calls.log.isEmpty)
    }

    @Test("what the app holds changing in the foreground changes nothing")
    internal func foregroundChanges() {
        enter(window)
        calls.holds = .nothing
        lifecycle.holdsChanged()
        #expect(calls.log.isEmpty)
        #expect(grace.begun == 0)
    }
}
