import QuartzCore

/// One row of a ``TerminalView``: its own layer, drawn again only when the row looks different.
///
/// Core Animation keeps what it drew, so a frame that changes a few rows draws only those. It draws on the main
/// thread, where the view lives.
internal final class TerminalRowLayer: CALayer {
    /// The row it draws.
    internal var index = 0
    /// The view whose row it draws.
    internal weak var view: TerminalView?

    /// A row layer that redraws as its size changes and never animates.
    override internal init() {
        super.init()
        isOpaque = true
        needsDisplayOnBoundsChange = true
    }

    /// The copy Core Animation makes of `layer` (a presentation layer).
    override internal init(layer: Any) {
        super.init(layer: layer)
        if let row = layer as? Self {
            index = row.index
        }
    }

    @available(*, unavailable, message: "a row layer is made in code")
    internal required init?(coder _: NSCoder) {
        nil
    }

    /// No implicit animations: a row's new contents, place or size show at once.
    override internal func action(forKey _: String) -> (any CAAction)? {
        nil
    }

    /// Draws the row as the view's screen has it now.
    override internal func draw(in context: CGContext) {
        let owner = view
        let row = index
        // Core Animation draws on the main thread, into a context it holds only for this call
        nonisolated(unsafe) let target = context
        MainActor.assumeIsolated {
            owner?.drawRow(row, in: target)
        }
    }

    deinit {
        // nothing to release
    }
}
