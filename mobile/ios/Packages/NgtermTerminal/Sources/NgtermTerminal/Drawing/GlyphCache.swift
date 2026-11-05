import CoreText
import Foundation

/// The shaped runs of each row, kept until the row changes (mobile/android's `GlyphCache`).
///
/// A run of the font's own glyphs is one line drawn at its first column. A run that would drift off the grid (a
/// fallback font's glyph of another width), a wide run and a cluster are drawn a glyph at a time, each centered in its
/// cells, or squeezed into them when wider. Shaping is the expensive part, and it happens once per row record and font.
/// The lines take their color from the context, so a row recolored (new default colors, inverse under the cursor) is
/// drawn from the same layout.
public final class GlyphCache {
    /// How far, in points, a run's measured width may stray from its cells before it is drawn glyph by glyph.
    private static let drift: CGFloat = 0.5
    private static let wideCells = 2
    private static let halves: CGFloat = 2

    private let font: TerminalFont
    private var layouts: [Int: RowLayout] = [:]

    /// A cache of rows laid out in `font`.
    public init(font: TerminalFont) {
        self.font = font
    }

    private static func line(_ text: String, face: CTFont) -> CTLine {
        let attributes: [NSAttributedString.Key: Any] = [
            NSAttributedString.Key(kCTFontAttributeName as String): face,
            NSAttributedString.Key(kCTForegroundColorFromContextAttributeName as String): true,
        ]
        return CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: attributes))
    }

    private static func width(of line: CTLine) -> CGFloat {
        CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
    }

    /// The layout of `row`: from the cache while `row` is the record it was made from.
    public func layout(_ row: TerminalRow) -> RowLayout {
        if let cached = layouts[row.index], cached.source === row {
            return cached
        }
        let made = RowLayout(source: row, draws: row.runs.flatMap(draws))
        layouts[row.index] = made
        return made
    }

    /// Forgets the layouts of rows from `rows` on: the grid shrank.
    public func trim(to rows: Int) {
        layouts = layouts.filter { $0.key < rows }
    }

    private func draws(_ run: TerminalRun) -> [GlyphDraw] {
        guard run.text.contains(where: { !$0.isWhitespace }) else {
            return []
        }
        let face = font.face(for: run.attributes)
        let left = font.left(run.column)
        if run.isCluster {
            return [glyph(run, run.text, at: left, cells: run.span, face: face)]
        }
        let glyphs = run.text.unicodeScalars.map { String($0) }
        if run.isWide {
            return glyphs.enumerated().map { index, text in
                glyph(run, text, at: left + font.left(index * Self.wideCells), cells: Self.wideCells, face: face)
            }
        }
        let line = Self.line(run.text, face: face)
        let expected = font.left(glyphs.count)
        if abs(Self.width(of: line) - expected) < Self.drift {
            return [GlyphDraw(run: run, line: line, left: left, scaleX: 1)]
        }
        return glyphs.enumerated().compactMap { index, text in
            text == " " ? nil : glyph(run, text, at: left + font.left(index), cells: 1, face: face)
        }
    }

    // `text` in `cells` cells from `left`: centered, or squeezed into them when wider
    private func glyph(_ run: TerminalRun, _ text: String, at left: CGFloat, cells: Int, face: CTFont) -> GlyphDraw {
        let line = Self.line(text, face: face)
        let room = font.left(cells)
        let width = Self.width(of: line)
        if width > room {
            return GlyphDraw(run: run, line: line, left: left, scaleX: room / width)
        }
        return GlyphDraw(run: run, line: line, left: left + (room - width) / Self.halves, scaleX: 1)
    }

    deinit {
        // nothing to release
    }
}
