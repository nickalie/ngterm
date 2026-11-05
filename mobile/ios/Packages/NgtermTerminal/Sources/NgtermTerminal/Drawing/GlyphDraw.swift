import CoreText

/// One draw of text: ``line`` with its left edge at ``left`` on the baseline, squeezed by ``scaleX``.
///
/// It draws (part of) ``run``, in the run's style; ``scaleX`` is 1 when the text fits its cells as it is.
public struct GlyphDraw {
    /// The run it draws (part of).
    public let run: TerminalRun
    /// The shaped text, in the run's face, its color taken from the context.
    public let line: CTLine
    /// Its left edge, in points from the row's.
    public let left: CGFloat
    /// The horizontal squeeze that fits it in its cells: 1 when it fits as it is.
    public let scaleX: CGFloat
}
