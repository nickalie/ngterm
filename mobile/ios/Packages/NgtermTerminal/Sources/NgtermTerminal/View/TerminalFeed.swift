/// Where a shown pane's frame events go while SwiftUI keeps its ``TerminalView``: the view it shows in now, if any.
///
/// The owner of a pane's terminal keeps one for as long as the pane shows, routes the pane's `PaneDirty` and
/// `PaneAttached` to it, and hands it to ``TerminalViewRepresentable``. A view SwiftUI makes anew reads its pane from
/// scratch, so nothing told before it came is lost.
@preconcurrency
@MainActor
public final class TerminalFeed {
    /// The view the pane shows in now.
    internal weak var view: TerminalView?

    /// A feed with no view yet.
    public init() {
        // the view joins as SwiftUI makes it
    }

    /// The pane has a new frame.
    public func dirty() {
        view?.dirty()
    }

    /// The pane has a fresh terminal: read it from scratch.
    public func attached() {
        view?.attached()
    }

    deinit {
        // nothing to release
    }
}
