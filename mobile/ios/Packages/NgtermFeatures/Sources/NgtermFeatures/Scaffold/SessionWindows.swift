import Observation

/// Where each session is drawn, shared by every scene: the one-view-per-session rule.
///
/// A session with a window of its own (iPad) opens nowhere else; opening it again focuses that window. A session
/// that main windows' detail columns show is drawn in one of them only, the last to open it: another main window
/// opening it takes it over, and the others step aside, since two views of one pane would fight over its size. When
/// that window lets it go, the one that had it before takes it back, so a session is never drawn twice, nor left
/// undrawn while a main window still shows it.
@preconcurrency
@MainActor
@Observable
public final class SessionWindows {
    /// The sessions with a window of their own.
    public private(set) var open: Set<SessionWindowID> = []
    // the main windows (their navigations' identities) showing each session, in the order they opened it: the last
    // one draws it, the others wait for it
    private var shownIn: [SessionWindowID: [ObjectIdentifier]] = [:]

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

    /// The main window `scene` now shows `session` (`nil`: none), taking it from any other main window; what it showed
    /// before goes back to the main window that had it last.
    internal func show(_ session: SessionWindowID?, in scene: ObjectIdentifier) {
        for (shown, scenes) in shownIn where scenes.contains(scene) {
            let rest = scenes.filter { $0 != scene }
            shownIn[shown] = rest.isEmpty ? nil : rest
        }
        if let session {
            shownIn[session, default: []].append(scene)
        }
    }

    /// Where `session` is drawn, as the main window `scene` sees it: here only while `scene` holds it.
    internal func placement(of session: SessionWindowID, in scene: ObjectIdentifier) -> SessionPlacement {
        if open.contains(session) {
            return .ownWindow
        }
        if let holder = shownIn[session]?.last, holder != scene {
            return .otherWindow
        }
        return .here
    }

    deinit {
        // nothing to release: the registry goes with the object
    }
}
