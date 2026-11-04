import Foundation
import NgtermCore
import NgtermCoreFakes
import Testing

/// The fake pane answers as the real terminal would where a view sees it (mobile/android's FakePane).
@Suite("the fake pane")
internal struct FakePaneTests {
    private static let point = CellPoint(x: 1, y: 2)

    @Test("frames come from what it packs, for the seq asked, and a pane with nothing to pack is closed")
    internal func framesComeFromItsPacker() throws {
        let pane = FakePane(paneId: "P1")
        #expect(throws: PaneInputError.Closed) {
            try pane.frame(sinceSeq: 0)
        }
        pane.frames = { since in Data([UInt8(since)]) }
        #expect(try pane.frame(sinceSeq: 7) == Data([7]))
        #expect(pane.asked == [0, 7])
        #expect(pane.id() == "P1")
    }

    @Test("a resize, new colors and a selection change wake the view, and are kept")
    internal func changesWakeTheView() throws {
        let woken = Counter()
        let pane = FakePane { _ in Data() }
        pane.onWake = { woken.add() }
        let size = PaneSize(cols: 80, rows: 24, cellWidth: 10, cellHeight: 20)
        #expect(pane.size() == PaneSize(cols: 0, rows: 0, cellWidth: 0, cellHeight: 0))
        pane.resize(size: size)
        #expect(pane.size() == size)
        let colors = PaneColors(foreground: 0xFF_FFFFFF, background: 0xFF_000000, cursor: nil, palette: [])
        pane.setColors(colors: colors)
        #expect(pane.colors == [colors])
        try pane.select(start: Self.point, end: Self.point, rectangle: false)
        #expect(try pane.selectWord(at: Self.point))
        try pane.clearSelection()
        #expect(woken.count == 5)
        #expect(
            pane.inputs == [
                .select(start: Self.point, end: Self.point, rectangle: false),
                .selectWord(Self.point),
                .clearSelection,
            ]
        )
    }

    @Test("a paste with a line break is refused unless allowed, and empty text writes nothing")
    internal func pasteFollowsThePasteProtection() throws {
        let pane = FakePane { _ in Data() }
        #expect(try pane.paste(text: "ls\n", allowUnsafe: false) == .unsafe)
        #expect(try pane.paste(text: "ls\n", allowUnsafe: true) == .written)
        #expect(try pane.paste(text: "", allowUnsafe: false) == .empty)
        #expect(pane.inputs.count == 3)
    }

    @Test("pointer events reach the program only while it tracks the mouse")
    internal func mouseReportsOnlyWhileTracking() throws {
        let pane = FakePane { _ in Data() }
        let press = PaneMouse(action: .press, button: .left, mods: 0, x: 4, y: 8)
        #expect(try !pane.mouse(mouse: press))
        pane.tracking = true
        #expect(try pane.mouse(mouse: press))
        #expect(pane.mouseTracking())
    }

    @Test("input to a pane with nothing to pack is refused as closed")
    internal func inputToAClosedPaneIsRefused() {
        let pane = FakePane()
        #expect(throws: PaneInputError.Closed) {
            try pane.typeText(text: "ls")
        }
        #expect(pane.inputs.isEmpty)
    }
}
