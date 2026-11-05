import CoreGraphics

/// The lines a run carries and the cursor's shapes, drawn on one row with its top at the origin (y down), at a
/// thickness that follows the text size (mobile/android's `TerminalDecorations`).
///
/// The lines are the strikethrough, the overline and the five underlines (single, double, curly, dotted, dashed), in
/// the underline color when the program set one.
public final class TerminalDecorations {
    /// A line is the text size over this: about Ghostty's underline at its sizes.
    private static let linePerSize: CGFloat = 14
    /// The thinnest line, in points.
    private static let thinnest: CGFloat = 0.5
    /// The strikethrough's height above the baseline, in text sizes: through the middle of the lowercase.
    private static let strikeHeight: CGFloat = 0.27
    /// How many lines under the baseline an underline sits.
    private static let underlineDrop: CGFloat = 2
    /// The lines' worth of room an underline leaves under it inside the cell: a double or curly one needs it.
    private static let underlineRoom: CGFloat = 2.5
    /// A dashed underline's dash, in cells over this: half a cell.
    private static let dashPerCell: CGFloat = 2
    /// A dashed underline's gap, in cells over this: a quarter cell after each dash.
    private static let gapPerCell: CGFloat = 4
    /// A curly underline's crest above and below its line, in lines.
    private static let waveHeight: CGFloat = 1.5
    /// A curly underline's half wave, in cells over this.
    private static let halfWavePerCell: CGFloat = 2
    /// A bar or underline cursor's thickness, in lines.
    private static let cursorLines: CGFloat = 2
    private static let halves: CGFloat = 2

    private let font: TerminalFont
    /// A line's thickness: the underline's, the cursor's bar, the hollow cursor's edge.
    private let line: CGFloat

    /// The lines and cursors of the cells of `font`.
    public init(font: TerminalFont) {
        self.font = font
        line = max(font.size / Self.linePerSize, Self.thinnest)
    }

    /// The lines of `run`, in `colors`.
    public func lines(of run: TerminalRun, colors: RunColors, in context: CGContext) {
        let left = font.left(run.column)
        let width = font.left(run.end) - left
        context.setFillColor(.argb(colors.foreground))
        if run.attributes.contains(.strikethrough) {
            let middle = font.baseline - font.size * Self.strikeHeight
            context.fill(CGRect(x: left, y: middle - line / Self.halves, width: width, height: line))
        }
        if run.attributes.contains(.overline) {
            context.fill(CGRect(x: left, y: 0, width: width, height: line))
        }
        guard let underline = run.underline else {
            return
        }
        let color = CGColor.argb(run.underlineColor == 0 ? colors.foreground : run.underlineColor)
        context.setFillColor(color)
        context.setStrokeColor(color)
        // under the baseline, inside the cell: a double or curly line needs room for two below it
        let middle = min(font.baseline + line * Self.underlineDrop, font.cellHeight - line * Self.underlineRoom)
        let span = left...(left + width)
        switch underline {
        case .curly:
            curly(span, at: middle, in: context)

        case .dashed:
            let pattern = (dash: font.cellWidth / Self.dashPerCell, gap: font.cellWidth / Self.gapPerCell)
            dashes(span, at: middle, pattern: pattern, in: context)

        case .dotted:
            dashes(span, at: middle, pattern: (dash: line, gap: line), in: context)

        case .double:
            context.fill(CGRect(x: left, y: middle - line, width: width, height: line))
            context.fill(CGRect(x: left, y: middle + line, width: width, height: line))

        case .single:
            context.fill(CGRect(x: left, y: middle - line / Self.halves, width: width, height: line))
        }
    }

    /// The cursor as `shape` over `cells` cells from `column`, in `color`; for a block, the rectangle it filled, where
    /// the text under it is to be drawn again in the cell's ground color, and `nil` for the other shapes.
    public func cursor(
        _ shape: CursorShape,
        column: Int,
        cells: Int,
        color: UInt32,
        in context: CGContext
    ) -> CGRect? {
        let left = font.left(column)
        let width = font.left(column + cells) - left
        let thickness = line * Self.cursorLines
        context.setFillColor(.argb(color))
        context.setStrokeColor(.argb(color))
        switch shape {
        case .bar:
            context.fill(CGRect(x: left, y: 0, width: thickness, height: font.cellHeight))

        case .block:
            let box = CGRect(x: left, y: 0, width: width, height: font.cellHeight)
            context.fill(box)
            return box

        case .hollowBlock:
            let box = CGRect(x: left, y: 0, width: width, height: font.cellHeight)
            context.setLineWidth(line)
            context.stroke(box.insetBy(dx: line / Self.halves, dy: line / Self.halves))

        case .underline:
            context.fill(CGRect(x: left, y: font.cellHeight - thickness, width: width, height: thickness))
        }
        return nil
    }

    private func curly(_ span: ClosedRange<CGFloat>, at middle: CGFloat, in context: CGContext) {
        let step = font.cellWidth / Self.halfWavePerCell
        let crest = line * Self.waveHeight
        context.beginPath()
        context.move(to: CGPoint(x: span.lowerBound, y: middle))
        var left = span.lowerBound
        var rising = true
        while left < span.upperBound {
            let right = min(left + step, span.upperBound)
            let control = CGPoint(x: (left + right) / Self.halves, y: rising ? middle - crest : middle + crest)
            context.addQuadCurve(to: CGPoint(x: right, y: middle), control: control)
            rising.toggle()
            left = right
        }
        context.setLineWidth(line)
        context.strokePath()
    }

    // dashes along `span` at `middle`, each `pattern.dash` long with `pattern.gap` between them
    private func dashes(
        _ span: ClosedRange<CGFloat>,
        at middle: CGFloat,
        pattern: (dash: CGFloat, gap: CGFloat),
        in context: CGContext
    ) {
        var left = span.lowerBound
        while left < span.upperBound {
            let right = min(left + pattern.dash, span.upperBound)
            context.fill(CGRect(x: left, y: middle - line / Self.halves, width: right - left, height: line))
            left += pattern.dash + pattern.gap
        }
    }

    deinit {
        // nothing to release
    }
}
