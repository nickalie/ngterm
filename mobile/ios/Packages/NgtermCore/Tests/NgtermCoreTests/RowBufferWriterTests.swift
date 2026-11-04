import Foundation
import NgtermCoreFakes
import Testing

/// The fakes' row buffer writer against the Rust writer's golden fixture (crates/ngterm-mobile/tests/fixtures/
/// rows-golden.bin, the screen rows/tests.rs's `GOLDEN_BYTES` draws on a 12 × 4 grid), which build-ios.sh copies into
/// the package's Generated/Fixtures/.
@Suite("the row buffer writer")
internal struct RowBufferWriterTests {
    private static let goldenURL = URL(filePath: #filePath)
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .appending(path: "Generated/Fixtures/rows-golden.bin")

    @Test("it packs the golden screen byte for byte as the Rust writer does")
    internal func packsTheGoldenScreenAsRustDoes() throws {
        let golden = try Data(contentsOf: Self.goldenURL)
        let cursor = FakeCursor(column: 10, row: 3, style: 1, blinking: true)
        let packed = RowBufferWriter(columns: 12, rows: 4, cursor: cursor)
            .row(
                0,
                FakeRun(0, "bold", attributes: FakeRun.bold, foreground: 0xFF_CC6666),
                FakeRun(4, " "),
                FakeRun(5, "ul", underline: 1, background: 0xFF_010203)
            )
            .row(
                1,
                FakeRun(0, "中", kind: FakeRun.wide),
                FakeRun(2, "e\u{301}", kind: FakeRun.cluster),
                FakeRun(3, "i", attributes: FakeRun.inverse),
                FakeRun(4, "\u{1F44D}\u{1F3FD}", kind: FakeRun.wide | FakeRun.cluster)
            )
            .row(2, FakeRun(0, "wrapping pas"), wrapped: true)
            .text(3, "t the edge")
            .select(0, first: 0, last: 3)
            .bytes()
        #expect(packed == golden)
    }

    @Test("a partial, stale frame with no cursor and nothing changed is its header and an empty selection")
    internal func partialStaleFrame() {
        let packed = RowBufferWriter(columns: 2, rows: 1, seq: 7, full: false, cursor: nil, stale: true).bytes()
        let header: [UInt8] = Array("NGRB".utf8) + [1, 0, 2, 0, 7, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0]
        #expect(packed.count == 42)
        #expect(Array(packed.prefix(header.count)) == header)
        // no cursor: position, style and flags all 0, then no rows and no selected rows
        #expect(packed.suffix(10).allSatisfy { $0 == 0 })
    }
}
