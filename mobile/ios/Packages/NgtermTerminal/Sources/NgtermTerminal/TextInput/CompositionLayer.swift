import CoreText
import QuartzCore

/// What an input method composes, drawn at the terminal's cursor until it commits (mobile/android's
/// `CompositionPainter`): the terminal's face on its ground, in its text color, underlined in the cursor's color.
internal final class CompositionLayer: CALayer {
    /// The underline's thickness, in points.
    internal static let underline: CGFloat = 2

    /// The text shaped, its color taken from the context; `nil` while nothing is composed.
    internal var line: CTLine?
    /// Where its baseline sits, from its top.
    internal var baseline: CGFloat = 0
    /// The terminal's ground under it.
    internal var ground = CGColor(gray: 0, alpha: 1)
    /// Its text color.
    internal var ink = CGColor(gray: 1, alpha: 1)
    /// The underline's color: the cursor's.
    internal var mark = CGColor(gray: 1, alpha: 1)

    /// A composition layer that never animates.
    override internal init() {
        super.init()
        isOpaque = true
        needsDisplayOnBoundsChange = true
    }

    /// The copy Core Animation makes of `layer` (a presentation layer).
    override internal init(layer: Any) {
        super.init(layer: layer)
    }

    @available(*, unavailable, message: "a composition layer is made in code")
    internal required init?(coder _: NSCoder) {
        nil
    }

    /// No implicit animations: the composition shows, moves and goes at once.
    override internal func action(forKey _: String) -> (any CAAction)? {
        nil
    }

    /// Draws the ground, the text and the underline.
    override internal func draw(in context: CGContext) {
        context.setFillColor(ground)
        context.fill(bounds)
        guard let line else {
            return
        }
        context.setFillColor(ink)
        // the context is y down: the text matrix stands the glyphs back up
        context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        context.textPosition = CGPoint(x: 0, y: baseline)
        CTLineDraw(line, context)
        context.setFillColor(mark)
        context.fill(CGRect(x: 0, y: bounds.height - Self.underline, width: bounds.width, height: Self.underline))
    }

    deinit {
        // nothing to release
    }
}
