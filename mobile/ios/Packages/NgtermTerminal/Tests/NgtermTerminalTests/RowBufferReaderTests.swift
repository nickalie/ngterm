import Foundation
import NgtermTerminal
import Testing

/// The reader against the Rust writer's golden fixture, as Android's `RowBufferReaderTest` reads it.
///
/// The fixture is crates/ngterm-mobile/tests/fixtures/rows-golden.bin, the screen rows/tests.rs's `GOLDEN_BYTES` draws
/// on a 12 × 4 grid, which build-ios.sh copies into NgtermCore's Generated/Fixtures/; the values checked are the ones
/// the Rust side's own tests of the fixture check.
@Suite("the row buffer reader")
internal struct RowBufferReaderTests {
    private struct Summary: Equatable, CustomStringConvertible {
        let column: Int
        let span: Int
        let kind: UInt8
        let underline: UInt8
        let attributes: UInt16
        let alpha: UInt32
        let text: String

        var description: String {
            "(\(column), \(span), \(kind), \(underline), \(attributes), \(alpha), \(text))"
        }

        init(
            _ column: Int,
            _ span: Int,
            _ kind: UInt8,
            _ underline: UInt8,
            _ attributes: UInt16,
            _ alpha: UInt32,
            _ text: String
        ) {
            self.column = column
            self.span = span
            self.kind = kind
            self.underline = underline
            self.attributes = attributes
            self.alpha = alpha
            self.text = text
        }

        init(_ run: TerminalRun) {
            self.init(
                run.column,
                run.span,
                run.kind,
                run.underline?.rawValue ?? 0,
                run.attributes.rawValue,
                run.foreground >> 24,
                run.text
            )
        }
    }

    private static let goldenURL = URL(filePath: #filePath)
        .deletingLastPathComponent()  // RowBufferReaderTests.swift
        .deletingLastPathComponent()  // NgtermTerminalTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // NgtermTerminal
        .appending(path: "NgtermCore/Generated/Fixtures/rows-golden.bin")

    private let golden: Data

    internal init() throws {
        golden = try Data(contentsOf: Self.goldenURL)
    }

    @Test("it reads the golden fixture's header, cursor and selection")
    internal func header() throws {
        let frame = try RowBufferReader.read(golden)
        let header = frame.header
        #expect(header.full)
        #expect(!header.stale)
        #expect(header.seq == 1)
        #expect(header.columns == 12)
        #expect(header.rows == 4)
        #expect(header.defaultForeground == 0xFFFF_FFFF)
        #expect(header.defaultBackground == 0xFF28_2C34)
        #expect(header.cursorColor == 0)
        let cursor = header.cursor
        #expect(cursor.column == 10)
        #expect(cursor.row == 3)
        #expect(cursor.shape == .bar)
        #expect(cursor.inView && cursor.visible && cursor.blinking)
        #expect(!cursor.password && !cursor.wideTail)
        #expect(cursor.shown)
        #expect(frame.selection == [SelectedSpan(row: 0, first: 0, last: 3)])
        #expect(frame.rows.map(\.index) == [0, 1, 2, 3])
    }

    @Test("it reads the golden fixture's rows: styles, wide runs, clusters and wraps")
    internal func rows() throws {
        let rows = try RowBufferReader.read(golden).rows
        #expect(
            rows[0].runs.map(Summary.init) == [
                Summary(0, 4, 0, 0, TerminalRun.Attributes.bold.rawValue, 0xFF, "bold"),
                Summary(4, 1, 0, 0, 0, 0, " "),
                Summary(5, 2, 0, TerminalRun.Underline.single.rawValue, 0, 0, "ul"),
            ]
        )
        #expect(rows[0].runs[0].foreground == 0xFFCC_6666)
        #expect(rows[0].runs[2].background == 0xFF01_0203)
        let wide = TerminalRun.wideKind
        let cluster = TerminalRun.clusterKind
        let inverse = TerminalRun.Attributes.inverse.rawValue
        #expect(
            rows[1].runs.map(Summary.init) == [
                Summary(0, 2, wide, 0, 0, 0, "中"),
                Summary(2, 1, cluster, 0, 0, 0, "e\u{301}"),
                Summary(3, 1, 0, 0, inverse, 0, "i"),
                Summary(4, 2, wide | cluster, 0, 0, 0, "\u{1F44D}\u{1F3FD}"),
            ]
        )
        #expect(rows[1].runs[0].isWide && !rows[1].runs[0].isCluster)
        #expect(rows[1].runs[3].isWide && rows[1].runs[3].isCluster)
        #expect(rows[2].wrapped)
        #expect(!rows[3].wrapped)
        #expect(rows[2].runs.map(Summary.init) == [Summary(0, 12, 0, 0, 0, 0, "wrapping pas")])
        #expect(rows[3].runs.map(Summary.init) == [Summary(0, 10, 0, 0, 0, 0, "t the edge")])
    }

    @Test("another magic is refused")
    internal func anotherMagic() {
        var bytes = golden
        bytes[0] = UInt8(ascii: "X")
        #expect(throws: MalformedFrame.notARowBuffer) { try RowBufferReader.read(bytes) }
    }

    @Test("another version is refused, naming it")
    internal func anotherVersion() {
        var bytes = golden
        bytes[4] = 2
        #expect(throws: MalformedFrame.version(2)) { try RowBufferReader.read(bytes) }
        #expect(MalformedFrame.version(2).description == "row buffer version 2, not 1")
    }

    @Test("a buffer cut short is refused", arguments: [0, 39, 40, -1])
    internal func cutShort(length: Int) {
        let bytes = golden.prefix(length < 0 ? golden.count + length : length)
        #expect(throws: MalformedFrame.cutShort) { try RowBufferReader.read(Data(bytes)) }
    }

    @Test("a buffer read from a slice reads as the whole one")
    internal func slice() throws {
        let padded = Data([0, 0, 0]) + golden
        let frame = try RowBufferReader.read(padded.dropFirst(3))
        #expect(frame.rows.count == 4)
        #expect(frame.selection.count == 1)
    }
}
