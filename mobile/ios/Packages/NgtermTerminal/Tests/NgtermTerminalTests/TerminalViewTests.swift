import NgtermCore
import NgtermCoreFakes
import NgtermDesign
import NgtermTerminal
import Testing
import UIKit

/// The view against a fake pane: the grid it gives the pane as its size changes, the colors, and how it reads frames.
@MainActor
@Suite("the terminal view")
internal struct TerminalViewTests {
    private static let textSize: CGFloat = 13
    private static let scale: CGFloat = 2

    private static func view(_ pane: FakePane, size: CGSize = CGSize(width: 390, height: 200)) -> TerminalView {
        let view = TerminalView(pane: pane, palette: .copper, textSize: textSize)
        view.traitOverrides.displayScale = scale
        view.updateTraitsIfNeeded()
        view.frame = CGRect(origin: .zero, size: size)
        view.layoutIfNeeded()
        return view
    }

    // the grid of whole cells of `font` that fits `size`
    private static func grid(_ size: CGSize, _ font: TerminalFont) -> PaneSize {
        PaneSize(
            cols: UInt16((size.width / font.cellWidth).rounded(.down)),
            rows: UInt16((size.height / font.cellHeight).rounded(.down)),
            cellWidth: UInt32((font.cellWidth * scale).rounded()),
            cellHeight: UInt32((font.cellHeight * scale).rounded())
        )
    }

    @Test("it gives the pane the grid that fits, again as its size changes, and once per size")
    internal func resize() {
        let pane = FakePane()
        let view = Self.view(pane)
        let upright = Self.grid(CGSize(width: 390, height: 200), view.font)
        #expect(pane.sizes == [upright])
        #expect(view.grid == upright)
        // a rotation, a Stage Manager window or the split gives it another size
        view.frame = CGRect(x: 0, y: 0, width: 844, height: 390)
        view.layoutIfNeeded()
        let sideways = Self.grid(CGSize(width: 844, height: 390), view.font)
        #expect(pane.sizes == [upright, sideways])
        // a layout at the same size sends nothing
        view.setNeedsLayout()
        view.layoutIfNeeded()
        #expect(pane.sizes.count == 2)
        // a size that holds no cell keeps the grid the pane has
        view.frame = CGRect(x: 0, y: 0, width: 3, height: 3)
        view.layoutIfNeeded()
        #expect(pane.sizes.count == 2)
        #expect(view.grid == sideways)
    }

    @Test("a new text size gives the pane a new grid")
    internal func textSize() {
        let pane = FakePane()
        let view = Self.view(pane)
        view.textSize = Self.textSize * 2
        view.layoutIfNeeded()
        #expect(view.font.size == Self.textSize * 2)
        #expect(pane.sizes.count == 2)
        #expect(pane.sizes.last == Self.grid(CGSize(width: 390, height: 200), view.font))
        #expect(pane.sizes[1].cols < pane.sizes[0].cols)
    }

    @Test("the cells fit the font: rows a whole number of points apart, columns its advance")
    internal func cells() {
        let font = TerminalFont(size: Self.textSize)
        #expect(font.cellHeight == font.cellHeight.rounded())
        #expect(font.cellHeight >= Self.textSize)
        #expect(font.baseline > 0 && font.baseline < font.cellHeight)
        #expect(font.cellWidth > 0 && font.cellWidth < font.cellHeight)
        #expect(font.grid(fitting: CGSize(width: font.cellWidth / 2, height: 100), scale: 1) == nil)
    }

    @Test("it gives the pane its palette's colors, and new ones as the palette changes")
    internal func colors() {
        let pane = FakePane()
        let view = Self.view(pane)
        #expect(pane.colors == [TerminalPalette.copper.paneColors])
        view.palette = .copper
        view.palette = .drafting
        #expect(pane.colors == [TerminalPalette.copper.paneColors, TerminalPalette.drafting.paneColors])
        let colors = TerminalPalette.ghostty.paneColors
        #expect(colors.background == 0xFF28_2C34)
        #expect(colors.foreground == 0xFFFF_FFFF)
        #expect(colors.palette.count == TerminalPalette.ansiCount)
        #expect(colors.palette[1] == 0xFFCC_6666)
    }

    @Test("it reads the first frame whole, then the rows changed since, and whole again after an attach")
    internal func since() {
        let pane = FakePane { since in RowBufferWriter(columns: 4, rows: 2, seq: since + 1).text(0, "seq").bytes() }
        let view = Self.view(pane)
        view.drawFrame()
        view.drawFrame()
        view.attached()
        view.drawFrame()
        #expect(pane.asked == [0, 1, 0])
        #expect(view.screen.text(0) == "seq")
    }

    @Test("it waits for a frame only while one is pending, and on after a stale one")
    internal func pending() {
        let stale = Locked(false)
        let pane = FakePane { _ in RowBufferWriter(columns: 4, rows: 2, stale: stale.value).bytes() }
        let view = Self.view(pane)
        #expect(view.framePending)
        view.drawFrame()
        #expect(!view.framePending)
        view.dirty()
        #expect(view.framePending)
        stale.value = true
        view.drawFrame()
        #expect(view.framePending)
        stale.value = false
        view.drawFrame()
        #expect(!view.framePending)
    }

    @Test("a pane with no terminal keeps the last screen on view")
    internal func noTerminal() {
        let pane = FakePane { _ in RowBufferWriter(columns: 8, rows: 2).text(1, "kept").bytes() }
        let view = Self.view(pane)
        view.drawFrame()
        pane.frames = nil
        view.dirty()
        view.drawFrame()
        #expect(view.screen.text(1) == "kept")
        #expect(!view.framePending)
    }

    @Test("a frame it can't read keeps the last screen and asks whole again")
    internal func malformed() {
        let broken = Locked(false)
        let pane = FakePane { _ in
            broken.value ? Data("NGRB".utf8) : RowBufferWriter(columns: 8, rows: 2).text(0, "ok").bytes()
        }
        let view = Self.view(pane)
        broken.value = true
        view.drawFrame()
        #expect(view.screen.rowCount == 0)
        broken.value = false
        view.dirty()
        view.drawFrame()
        #expect(pane.asked == [0, 0])
        #expect(view.screen.text(0) == "ok")
    }
}
