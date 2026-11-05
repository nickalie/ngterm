import NgtermCore

/// What a pane's show waits on: the pane, the room its slot has, and whether it still waits to be shown.
internal struct PaneShowKey: Equatable {
    /// The pane.
    internal let ref: PaneRef
    /// Its slot's width, in points.
    internal let width: Double
    /// Its slot's height, in points.
    internal let height: Double
    /// Whether it waits to be shown: not shown, and no failure since the last try.
    internal let waits: Bool
}
