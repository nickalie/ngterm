import Observation

/// The sessions that have a window of their own (iPad), shared by every scene: the one-window-per-session rule.
///
/// A session with its own window opens nowhere else; opening it again focuses that window.
@preconcurrency
@MainActor
@Observable
public final class SessionWindows {
    /// The sessions with a window of their own.
    public private(set) var open: Set<SessionWindowID> = []

    /// No session has a window of its own yet.
    public init() {
        // starts empty: each session window reports itself as it appears
    }

    /// A session's own window appeared.
    public func opened(_ session: SessionWindowID) {
        open.insert(session)
    }

    /// A session's own window went away (closed, or its scene discarded).
    public func closed(_ session: SessionWindowID) {
        open.remove(session)
    }

    /// Where `session` opens when a window's list asks for it.
    public func placement(of session: SessionWindowID) -> SessionPlacement {
        open.contains(session) ? .ownWindow : .here
    }

    deinit {
        // nothing to release: the set goes with the object
    }
}
