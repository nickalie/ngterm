import Observation

/// Where each session is drawn, shared by every scene: the one-view-per-session rule.
///
/// A session with a window of its own (iPad) opens nowhere else; opening it again focuses that window. A session
/// that a main window's detail column shows is that window's: another main window opening it takes it over, and the
/// first one steps aside, since two views of one pane would fight over its size.
@preconcurrency
@MainActor
@Observable
public final class SessionWindows {
    /// The sessions with a window of their own.
    public private(set) var open: Set<SessionWindowID> = []
    // the main window (its navigation's identity) each session shows in
    private var shownIn: [SessionWindowID: ObjectIdentifier] = [:]

    /// No session has a window of its own yet, and no main window shows one.
    public init() {
        // starts empty: each session window reports itself as it appears, each main window as it opens a session
    }

    /// A session's own window appeared.
    public func opened(_ session: SessionWindowID) {
        open.insert(session)
    }

    /// A session's own window went away (closed, or its scene discarded).
    public func closed(_ session: SessionWindowID) {
        open.remove(session)
    }

    /// The main window `scene` now shows `session` (`nil`: none), taking it from any other main window.
    internal func show(_ session: SessionWindowID?, in scene: ObjectIdentifier) {
        shownIn = shownIn.filter { $0.value != scene }
        if let session {
            shownIn[session] = scene
        }
    }

    /// Where `session` is drawn, as the main window `scene` sees it.
    internal func placement(of session: SessionWindowID, in scene: ObjectIdentifier) -> SessionPlacement {
        if open.contains(session) {
            return .ownWindow
        }
        if let shower = shownIn[session], shower != scene {
            return .otherWindow
        }
        return .here
    }

    deinit {
        // nothing to release: the registry goes with the object
    }
}
