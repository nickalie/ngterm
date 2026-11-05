import QuartzCore

/// A touch selection's two handles, drawn over the rows of a ``TerminalView``.
///
/// Each is a stem two points wide on the selection's edge and a round knob, in the palette's cursor color (the
/// terminal's own accent).
internal final class SelectionHandles: CAShapeLayer {
    private static let stemWidth: CGFloat = 2
    private static let halves: CGFloat = 2

    /// Where the handles stand; `nil` hides them.
    internal private(set) var spots: HandleSpots?

    /// Handles drawn in no color yet, hidden.
    override internal init() {
        super.init()
        isHidden = true
    }

    /// The copy Core Animation makes of `layer` (a presentation layer).
    override internal init(layer: Any) {
        super.init(layer: layer)
    }

    @available(*, unavailable, message: "the handles are made in code")
    internal required init?(coder _: NSCoder) {
        nil
    }

    /// No implicit animations: the handles follow the selection at once.
    override internal func action(forKey _: String) -> (any CAAction)? {
        nil
    }

    /// Shows the handles at `spots`, in `color`; `nil` hides them.
    internal func show(_ spots: HandleSpots?, color: CGColor) {
        self.spots = spots
        fillColor = color
        guard let spots else {
            isHidden = true
            path = nil
            return
        }
        let shape = CGMutablePath()
        let radius = HandleSpots.knobRadius
        for handle in [spots.start, spots.end] {
            let stem = CGRect(
                x: handle.stemTop.x - Self.stemWidth / Self.halves,
                y: handle.stemTop.y,
                width: Self.stemWidth,
                height: handle.stemBottom.y - handle.stemTop.y
            )
            shape.addRect(stem)
            let diameter = radius * Self.halves
            shape.addEllipse(
                in: CGRect(x: handle.knob.x - radius, y: handle.knob.y - radius, width: diameter, height: diameter)
            )
        }
        path = shape
        isHidden = false
    }

    deinit {
        // nothing to release
    }
}
