import Foundation
import NgtermCoreFakes
import NgtermTerminal
import Testing

/// The screen the view draws: the rows it holds as frames come, and which rows each frame makes it draw again.
@Suite("the terminal screen")
internal struct TerminalScreenTests {
    private static func update(_ writer: RowBufferWriter) throws -> FrameUpdate {
        try RowBufferReader.read(writer.bytes())
    }

    private static func screen(_ writer: RowBufferWriter) throws -> TerminalScreen {
        var screen = TerminalScreen()
        _ = screen.apply(try update(writer))
        return screen
    }

    @Test("a first frame draws every row, the ones it didn't bring blank")
    internal func firstFrame() throws {
        var screen = TerminalScreen()
        #expect(screen.seq == 0)
        let changed = screen.apply(try Self.update(RowBufferWriter(columns: 10, rows: 3).text(1, "hello")))
        #expect(Array(changed) == [0, 1, 2])
        #expect(screen.rowCount == 3)
        #expect(screen.columns == 10)
        #expect(screen.seq == 1)
        #expect(screen.row(0) == nil)
        #expect(screen.text(1) == "hello")
    }

    @Test("a partial frame draws the rows it brings and keeps the others as they were")
    internal func partialFrame() throws {
        var screen = try Self.screen(RowBufferWriter(columns: 10, rows: 3).text(0, "one").text(2, "three"))
        let kept = screen.row(0)
        let next = RowBufferWriter(columns: 10, rows: 3, seq: 2, full: false).text(2, "3")
        let changed = screen.apply(try Self.update(next))
        #expect(Array(changed) == [2])
        #expect(screen.row(0) === kept)
        #expect(screen.text(2) == "3")
        #expect(screen.seq == 2)
    }

    @Test("a full frame starts from blank rows")
    internal func fullFrame() throws {
        var screen = try Self.screen(RowBufferWriter(columns: 10, rows: 2).text(0, "old"))
        let changed = screen.apply(try Self.update(RowBufferWriter(columns: 10, rows: 2, seq: 5).text(1, "new")))
        #expect(Array(changed) == [0, 1])
        #expect(screen.row(0) == nil)
        #expect(screen.text(1) == "new")
    }

    @Test("a new grid starts from blank rows, and a row past it is dropped")
    internal func newGrid() throws {
        var screen = try Self.screen(RowBufferWriter(columns: 10, rows: 2).text(0, "old"))
        let next = RowBufferWriter(columns: 10, rows: 3, seq: 2, full: false).text(1, "in").text(7, "past")
        let changed = screen.apply(try Self.update(next))
        #expect(Array(changed) == [0, 1, 2])
        #expect(screen.row(0) == nil)
        #expect(screen.text(1) == "in")
        #expect(screen.row(7) == nil)
    }

    @Test("a selection change draws the rows it left and the rows it reached")
    internal func selection() throws {
        let first = RowBufferWriter(columns: 10, rows: 4).select(0, first: 0, last: 3).select(1, first: 0, last: 9)
        var screen = try Self.screen(first)
        let next = RowBufferWriter(columns: 10, rows: 4, seq: 2, full: false)
            .select(1, first: 0, last: 9)
            .select(2, first: 0, last: 1)
        let changed = screen.apply(try Self.update(next))
        #expect(Array(changed) == [0, 2])
        #expect(screen.selection(of: 2) == SelectedSpan(row: 2, first: 0, last: 1))
        #expect(screen.selection(of: 0) == nil)
        #expect(screen.selection == [SelectedSpan(row: 1, first: 0, last: 9), SelectedSpan(row: 2, first: 0, last: 1)])
    }

    @Test("a cursor that moves draws its old row and its new one")
    internal func cursorMoves() throws {
        var screen = try Self.screen(RowBufferWriter(columns: 10, rows: 4, cursor: FakeCursor(column: 2, row: 0)))
        let next = RowBufferWriter(columns: 10, rows: 4, seq: 2, full: false, cursor: FakeCursor(column: 0, row: 3))
        #expect(Array(screen.apply(try Self.update(next))) == [0, 3])
        let same = RowBufferWriter(columns: 10, rows: 4, seq: 3, full: false, cursor: FakeCursor(column: 0, row: 3))
        #expect(screen.apply(try Self.update(same)).isEmpty)
    }

    @Test("a cursor that changes shape or color draws its row")
    internal func cursorLooks() throws {
        var screen = try Self.screen(RowBufferWriter(columns: 10, rows: 4, cursor: FakeCursor(column: 2, row: 1)))
        let barCursor = FakeCursor(column: 2, row: 1, style: 1)
        let bar = RowBufferWriter(columns: 10, rows: 4, seq: 2, full: false, cursor: barCursor)
        #expect(Array(screen.apply(try Self.update(bar))) == [1])
        let colored = RowBufferWriter(columns: 10, rows: 4, seq: 3, full: false, cursor: barCursor)
            .colors(
                foreground: RowBufferWriter.ghosttyForeground,
                background: RowBufferWriter.ghosttyBackground,
                cursor: 0xFF11_2233
            )
        #expect(Array(screen.apply(try Self.update(colored))) == [1])
        #expect(screen.header?.cursorColor == 0xFF11_2233)
    }

    @Test("new default colors draw every row: a run's defaults are resolved as it is drawn")
    internal func newDefaults() throws {
        var screen = try Self.screen(RowBufferWriter(columns: 10, rows: 3).text(0, "kept"))
        let next = RowBufferWriter(columns: 10, rows: 3, seq: 2, full: false)
            .colors(foreground: 0xFF00_0000, background: 0xFFFF_FFFF)
        #expect(Array(screen.apply(try Self.update(next))) == [0, 1, 2])
        #expect(screen.text(0) == "kept")
        #expect(screen.header?.defaultBackground == 0xFFFF_FFFF)
    }

    @Test("a row's text has its blank cells as spaces and no trailing blanks")
    internal func text() throws {
        let runs = [FakeRun(0, "$"), FakeRun(2, "ls  "), FakeRun(8, "中", kind: FakeRun.wide)]
        let screen = try Self.screen(RowBufferWriter(columns: 20, rows: 1).row(0, runs))
        #expect(screen.text(0) == "$ ls    中")
        #expect(screen.text(5).isEmpty)
    }
}
