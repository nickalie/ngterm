/// The app's life across all its scenes (mobile/android's `Foreground` and `IdleKeeper`): when it lets go of its
/// connections in the background, and brings them back.
///
/// It counts the scenes in the foreground, so one iPad window going to the background changes nothing while another
/// stays. Once the last one has left (and only after one was shown: a background launch does nothing), it asks what
/// the app holds (``LifecycleHold``):
/// - nothing: it suspends at once;
/// - attached sessions: it keeps a ``BackgroundGrace``, and suspends once it runs out;
/// - a machine waiting for the user to sign in: it doesn't suspend, for the connect must still be there when they have.
///
/// What the app holds can change in the background (a sign-in ends); ``holdsChanged()`` looks again. A scene back in
/// the foreground ends the grace, and resumes what a suspend let go.
@preconcurrency
@MainActor
public final class Lifecycle {
    /// Whether the app let go of its connections, until a scene comes back.
    public private(set) var isSuspended = false
    private let grace: any BackgroundGrace
    private let holds: @MainActor () -> LifecycleHold
    private let suspend: @MainActor () -> Void
    private let resume: @MainActor () -> Void
    private var foreground: Set<ObjectIdentifier> = []
    private var wasShown = false
    private var isGraceRunning = false

    /// Whether a scene of the app is in the foreground.
    public var isInForeground: Bool {
        !foreground.isEmpty
    }

    /// The life of an app that keeps `grace` in the background, asks `holds` what it holds, and lets go of its
    /// connections by `suspend` and brings them back by `resume`.
    @preconcurrency
    public init(
        grace: any BackgroundGrace,
        holds: @escaping @MainActor () -> LifecycleHold,
        suspend: @escaping @MainActor () -> Void,
        resume: @escaping @MainActor () -> Void
    ) {
        self.grace = grace
        self.holds = holds
        self.suspend = suspend
        self.resume = resume
    }

    /// The scene `scene` came to the foreground: the grace ends, and what a suspend let go comes back.
    public func sceneEntered(_ scene: ObjectIdentifier) {
        foreground.insert(scene)
        wasShown = true
        endGrace()
        if isSuspended {
            isSuspended = false
            resume()
        }
    }

    /// The scene `scene` left the foreground, or went: once none is left, the app lets go as it holds.
    public func sceneLeft(_ scene: ObjectIdentifier) {
        foreground.remove(scene)
        settle()
    }

    /// What the app holds changed (its attached sessions, a sign-in): in the background, it looks again.
    public func holdsChanged() {
        settle()
    }

    private func settle() {
        guard wasShown, foreground.isEmpty, !isSuspended else {
            return
        }
        switch holds() {
        case .nothing:
            endGrace()
            suspendNow()

        case .signIn:
            endGrace()

        case .sessions:
            guard !isGraceRunning else {
                return
            }
            isGraceRunning = true
            grace.begin { [weak self] in
                self?.graceRanOut()
            }
        }
    }

    private func graceRanOut() {
        isGraceRunning = false
        guard foreground.isEmpty, !isSuspended, holds() != .signIn else {
            return
        }
        suspendNow()
    }

    private func endGrace() {
        guard isGraceRunning else {
            return
        }
        isGraceRunning = false
        grace.end()
    }

    private func suspendNow() {
        isSuspended = true
        suspend()
    }

    deinit {
        // nothing to release: the grace ends with the process
    }
}
