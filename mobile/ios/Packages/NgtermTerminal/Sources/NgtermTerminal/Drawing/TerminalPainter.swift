import CoreGraphics
import CoreText

/// Draws one row of a ``TerminalScreen`` with its top left at the origin (y down), as Ghostty's renderer draws a frame
/// (mobile/android's `TerminalPainter`, a row at a time).
///
/// - the ground in the frame's default background, then each run's own background;
/// - the selection, in the palette's selection color, under the text, which keeps its colors;
/// - the text, from the row's ``GlyphCache`` layout: bold and italic pick the face, faint halves the alpha, inverse
///   swaps the colors, invisible draws nothing;
/// - the lines and the cursor (``TerminalDecorations``).
///
/// A run's default colors (`0`) are read from the header being drawn, never from an earlier one, so a change of the
/// defaults (OSC 10/11) recolors rows that were not sent again.
public final class TerminalPainter {
    private static let wideCells = 2

    private let font: TerminalFont
    private let glyphs: GlyphCache
    private let decorations: TerminalDecorations

    /// A painter of rows in `font`, laid out through `glyphs`.
    public init(font: TerminalFont, glyphs: GlyphCache) {
        self.font = font
        self.glyphs = glyphs
        decorations = TerminalDecorations(font: font)
    }

    /// Draws row `index` of `screen`, `width` points wide, in `look`, into `context`; nothing before the first frame.
    public func draw(
        row index: Int,
        of screen: TerminalScreen,
        width: CGFloat,
        look: TerminalLook,
        in context: CGContext
    ) {
        guard let header = screen.header else {
            return
        }
        context.setFillColor(.argb(header.defaultBackground))
        context.fill(CGRect(x: 0, y: 0, width: width, height: font.cellHeight))
        let row = screen.row(index)
        if let row {
            backgrounds(of: row, under: header, in: context)
        }
        if let span = screen.selection(of: index) {
            let left = font.left(span.first)
            context.setFillColor(.argb(look.selection))
            context.fill(CGRect(x: left, y: 0, width: font.left(span.last + 1) - left, height: font.cellHeight))
        }
        if let row {
            text(of: row, under: header, in: context)
            for run in row.runs where !run.attributes.contains(.invisible) {
                decorations.lines(of: run, colors: RunColors(of: run, under: header), in: context)
            }
        }
        if header.cursor.shown, header.cursor.row == index {
            cursor(on: row, under: header, focused: look.focused, in: context)
        }
    }

    private func backgrounds(of row: TerminalRow, under header: TerminalHeader, in context: CGContext) {
        for run in row.runs {
            let ground = RunColors(of: run, under: header).background
            guard ground != header.defaultBackground else {
                continue
            }
            let left = font.left(run.column)
            context.setFillColor(.argb(ground))
            context.fill(CGRect(x: left, y: 0, width: font.left(run.end) - left, height: font.cellHeight))
        }
    }

    // the row's text, each run in its colors, or all of it in `color`
    private func text(of row: TerminalRow, under header: TerminalHeader, in context: CGContext, color: UInt32? = nil) {
        for draw in glyphs.layout(row).draws where !draw.run.attributes.contains(.invisible) {
            context.setFillColor(.argb(color ?? RunColors(of: draw.run, under: header).foreground))
            // the context is y down: the text matrix stands the glyphs back up
            context.textMatrix = CGAffineTransform(scaleX: draw.scaleX, y: -1)
            context.textPosition = CGPoint(x: draw.left, y: font.baseline)
            CTLineDraw(draw.line, context)
        }
    }

    private func cursor(on row: TerminalRow?, under header: TerminalHeader, focused: Bool, in context: CGContext) {
        let cursor = header.cursor
        // on a wide character the cursor covers both its columns, from the first
        let column = cursor.drawnColumn
        let under = row?.run(at: column)
        let cell = under.map { RunColors(of: $0, under: header) } ?? RunColors(defaultsOf: header)
        let color = header.cursorColor == 0 ? cell.foreground : header.cursorColor
        let shape = focused ? cursor.shape : .hollowBlock
        let cells = under?.isWide == true ? Self.wideCells : 1
        let box = decorations.cursor(shape, column: column, cells: cells, color: color, in: context)
        guard let box, let row else {
            return
        }
        // the character under a block reads in the cell's ground color
        context.saveGState()
        context.clip(to: box)
        text(of: row, under: header, in: context, color: cell.background)
        context.restoreGState()
    }

    deinit {
        // nothing to release
    }
}
