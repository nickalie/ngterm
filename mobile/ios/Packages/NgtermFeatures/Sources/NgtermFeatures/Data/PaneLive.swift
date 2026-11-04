import NgtermCore

/// A shown pane as its events left it (mobile/android's `PaneLive`).
public struct PaneLive: Hashable, Sendable {
    /// Its role at the session: `nil` until its attach reports one.
    public var role: PaneRole?
    /// Whether it is covered: a follower's, or one attaching again with nothing to show yet.
    public var isCovered: Bool
    /// How its attach ended: `nil` while it runs.
    public var end: PaneEnd?
    /// The title its program set.
    public var title: String?

    /// A pane with its `role`, cover, `end` and `title`.
    public init(role: PaneRole? = nil, isCovered: Bool = false, end: PaneEnd? = nil, title: String? = nil) {
        self.role = role
        self.isCovered = isCovered
        self.end = end
        self.title = title
    }
}
