import CoreGraphics
import NgtermCore
import NgtermTerminal
import Testing

/// Where a touch selection's handles stand, and which one a finger takes.
@MainActor
@Suite("the selection's handles")
internal struct HandleSpotsTests {
    private static let metrics = TerminalFont(size: 13)
    private static let size = CGSize(width: 390, height: 200)

    @Test("the start's stem is on the first cell's left, the end's on the last cell's right, each a row tall")
    internal func stems() throws {
        let font = Self.metrics
        let spots = try #require(
            HandleSpots(
                [SelectedSpan(row: 3, first: 6, last: 20), SelectedSpan(row: 2, first: 4, last: 39)],
                font: font,
                size: Self.size
            )
        )
        #expect(spots.start.cell == CellPoint(x: 4, y: 2))
        #expect(spots.start.fixed == CellPoint(x: 20, y: 3))
        #expect(spots.end.cell == CellPoint(x: 20, y: 3))
        #expect(spots.end.fixed == CellPoint(x: 4, y: 2))
        #expect(spots.start.stemTop == CGPoint(x: font.left(4), y: font.top(2)))
        #expect(spots.start.stemBottom == CGPoint(x: font.left(4), y: font.top(3)))
        #expect(spots.end.stemTop == CGPoint(x: font.left(21), y: font.top(3)))
        // the start's knob above its stem, the end's below
        #expect(spots.start.knob.y == font.top(2) - HandleSpots.knobRadius)
        #expect(spots.end.knob.y == font.top(4) + HandleSpots.knobRadius)
        #expect(spots.rows == CGRect(x: 0, y: font.top(2), width: Self.size.width, height: 2 * font.cellHeight))
        let center = CGPoint(x: font.left(4) + font.cellWidth / 2, y: font.top(2) + font.cellHeight / 2)
        #expect(spots.start.cellCenter == center)
    }

    @Test("a knob with no room on its side of the stem hangs on the other")
    internal func knobsAtTheEdges() throws {
        let font = Self.metrics
        let height = font.top(5)
        let spots = try #require(
            HandleSpots(
                [SelectedSpan(row: 0, first: 0, last: 2), SelectedSpan(row: 4, first: 0, last: 2)],
                font: font,
                size: CGSize(width: Self.size.width, height: height)
            )
        )
        #expect(spots.start.knob.y == font.top(1) + HandleSpots.knobRadius)
        #expect(spots.end.knob.y == font.top(4) - HandleSpots.knobRadius)
    }

    @Test("a finger takes the nearer handle within reach of its knob or stem, and none farther off")
    internal func nearest() throws {
        let selection = [SelectedSpan(row: 5, first: 2, last: 30)]
        let spots = try #require(HandleSpots(selection, font: Self.metrics, size: Self.size))
        let start = spots.start.knob
        let end = spots.end.knob
        #expect(spots.nearest(to: CGPoint(x: start.x + 10, y: start.y - 10)) == spots.start)
        #expect(spots.nearest(to: CGPoint(x: end.x - 15, y: end.y + 5)) == spots.end)
        #expect(spots.nearest(to: CGPoint(x: end.x, y: spots.end.stemTop.y + 2)) == spots.end)
        #expect(spots.nearest(to: CGPoint(x: end.x + HandleSpots.reach + 1, y: end.y)) == nil)
        #expect(spots.nearest(to: CGPoint(x: (start.x + end.x) / 2, y: start.y)) == nil)
    }

    @Test("no selection has no handles")
    internal func none() {
        #expect(HandleSpots([], font: Self.metrics, size: Self.size) == nil)
    }
}
