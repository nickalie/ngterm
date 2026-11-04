/// Where a shown pane's new frames go: its terminal view, told on the main actor.
@preconcurrency
@MainActor
public protocol PaneFrames: AnyObject {
    /// The pane has a new frame.
    func dirty()

    /// The pane has a fresh terminal: read it from scratch.
    func attached()
}
