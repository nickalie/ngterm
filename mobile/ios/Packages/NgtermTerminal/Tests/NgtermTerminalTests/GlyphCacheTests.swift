import CoreText
import NgtermCoreFakes
import NgtermTerminal
import Testing

/// The shaped runs of each row: kept while the row is the same record, and laid on the grid glyph by glyph where one
/// line would drift off it.
@Suite("the glyph cache")
internal struct GlyphCacheTests {
    private let font = TerminalFont(size: 13)

    private static func row(_ runs: FakeRun...) throws -> TerminalRow {
        let frame = try RowBufferReader.read(RowBufferWriter(columns: 40, rows: 1).row(0, runs).bytes())
        return try #require(frame.rows.first)
    }

    private static func width(of draw: GlyphDraw) -> CGFloat {
        CGFloat(CTLineGetTypographicBounds(draw.line, nil, nil, nil)) * draw.scaleX
    }

    @Test("a row is laid out once per record: the same record comes from the cache, a row sent again anew")
    internal func cached() throws {
        let cache = GlyphCache(font: font)
        let row = try Self.row(FakeRun(0, "hello"))
        let first = cache.layout(row)
        let again = cache.layout(row)
        #expect(first.source === row)
        #expect(zip(first.draws, again.draws).allSatisfy { $0.line === $1.line })
        let resent = try Self.row(FakeRun(0, "hello"))
        let fresh = cache.layout(resent)
        #expect(fresh.source === resent)
        #expect(!zip(first.draws, fresh.draws).contains { $0.line === $1.line })
    }

    @Test("a run of the font's own glyphs is one line at its first column")
    internal func ownGlyphs() throws {
        let layout = GlyphCache(font: font).layout(try Self.row(FakeRun(3, "make"), FakeRun(8, "ok")))
        #expect(layout.draws.map(\.left) == [font.left(3), font.left(8)])
        #expect(layout.draws.allSatisfy { $0.scaleX == 1 })
    }

    @Test("a blank run draws nothing")
    internal func blank() throws {
        let layout = GlyphCache(font: font).layout(try Self.row(FakeRun(0, "    ", background: 0xFF00_0000)))
        #expect(layout.draws.isEmpty)
    }

    @Test("a wide run is a glyph per two cells, each within them")
    internal func wide() throws {
        let layout = GlyphCache(font: font).layout(try Self.row(FakeRun(2, "中文", kind: FakeRun.wide)))
        #expect(layout.draws.count == 2)
        for (index, draw) in layout.draws.enumerated() {
            let cells = font.left(2 + index * 2)...font.left(4 + index * 2)
            #expect(cells.contains(draw.left))
            #expect(draw.left + Self.width(of: draw) <= cells.upperBound + 0.001)
        }
    }

    @Test("a narrow run with a glyph wider than its cell is drawn glyph by glyph, the wide one squeezed into its cell")
    internal func drift() throws {
        let layout = GlyphCache(font: font).layout(try Self.row(FakeRun(0, "a 中b")))
        #expect(layout.draws.count == 3)
        #expect(layout.draws.map(\.scaleX).filter { $0 < 1 }.count == 1)
        let squeezed = layout.draws[1]
        #expect(squeezed.left == font.left(2))
        #expect(abs(Self.width(of: squeezed) - font.cellWidth) < 0.001)
        let last = layout.draws[2]
        #expect((font.left(3)...font.left(4)).contains(last.left))
    }
}
