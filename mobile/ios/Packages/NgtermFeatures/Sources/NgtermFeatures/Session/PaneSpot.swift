import NgtermCore

/// One pane of a session as the session view places it (mobile/android's `PaneSpot`): which pane, its terminal once
/// it is shown, what its events say, and why it could not be shown.
@preconcurrency
@MainActor
public struct PaneSpot {
    /// Which pane.
    public let ref: PaneRef
    /// Its terminal, once it is shown.
    public let shown: ShownPane?
    /// What its events say.
    public let live: PaneLive?
    /// Why it could not be shown (the core's words).
    public let failure: String?

    /// The pane's id in its session.
    public var id: String {
        ref.paneId
    }

    /// What covers it, if anything.
    public var cover: PaneCover? {
        PaneCover.of(live, hasTerminal: shown != nil, failure: failure)
    }

    /// The pane `ref`, `shown` or not yet, with its `live` state and its show's `failure`.
    public init(ref: PaneRef, shown: ShownPane?, live: PaneLive?, failure: String?) {
        self.ref = ref
        self.shown = shown
        self.live = live
        self.failure = failure
    }
}
