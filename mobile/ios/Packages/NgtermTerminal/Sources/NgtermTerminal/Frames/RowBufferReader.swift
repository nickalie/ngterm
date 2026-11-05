import Foundation

/// Reads the packed row buffer `Pane.frame` returns, as crates/ngterm-mobile/src/rows.rs's module docs lay it out
/// (version 1, magic `NGRB`, little-endian).
///
/// Its test decodes the Rust writer's golden fixture (crates/ngterm-mobile/tests/fixtures/rows-golden.bin), which keeps
/// the two in step, as it does Android's reader (mobile/android's `RowBufferReader`).
public enum RowBufferReader {
    /// The layout's version this reader follows.
    public static let version: UInt16 = 1

    private static let magic = Array("NGRB".utf8)
    private static let headerBytes = 40
    private static let rowFixedBytes = 5
    private static let runFixedBytes = 24
    private static let spanBytes = 6
    private static let frameFull: UInt16 = 1 << 0
    private static let frameStale: UInt16 = 1 << 1
    private static let rowWrapped: UInt8 = 1 << 0

    /// Decodes `data`.
    ///
    /// - Throws: ``MalformedFrame`` for another magic or version, or a buffer cut short.
    public static func read(_ data: Data) throws(MalformedFrame) -> FrameUpdate {
        let result = data.withUnsafeBytes { bytes in
            Result { () throws(MalformedFrame) in
                var reader = ByteReader(bytes)
                return try read(&reader)
            }
        }
        return try result.get()
    }

    private static func read(_ reader: inout ByteReader) throws(MalformedFrame) -> FrameUpdate {
        let header = try header(&reader)
        let rowCount = try reader.u16()
        var rows: [TerminalRow] = []
        rows.reserveCapacity(rowCount)
        for _ in 0..<rowCount {
            rows.append(try row(&reader))
        }
        let spans = try reader.u16()
        try reader.need(spans * spanBytes)
        var selection: [SelectedSpan] = []
        selection.reserveCapacity(spans)
        for _ in 0..<spans {
            selection.append(SelectedSpan(row: try reader.u16(), first: try reader.u16(), last: try reader.u16()))
        }
        return FrameUpdate(header: header, rows: rows, selection: selection)
    }

    // the header up to the row count that ends it
    private static func header(_ reader: inout ByteReader) throws(MalformedFrame) -> TerminalHeader {
        try reader.need(headerBytes)
        guard Array(try reader.bytes(magic.count)) == magic else {
            throw MalformedFrame.notARowBuffer
        }
        let found = try reader.number(UInt16.self)
        guard found == version else {
            throw MalformedFrame.version(found)
        }
        let flags = try reader.number(UInt16.self)
        let seq = try reader.u64()
        let grid = (columns: try reader.u16(), rows: try reader.u16())
        let defaults = (foreground: try reader.u32(), background: try reader.u32())
        let cursorColor = try reader.u32()
        let column = try reader.u16()
        let row = try reader.u16()
        let shape = CursorShape(style: try reader.u8())
        let cursor = TerminalCursor(column: column, row: row, shape: shape, flags: try reader.u8())
        return TerminalHeader(
            seq: seq,
            full: flags & frameFull != 0,
            stale: flags & frameStale != 0,
            grid: grid,
            defaults: defaults,
            cursorColor: cursorColor,
            cursor: cursor
        )
    }

    private static func row(_ reader: inout ByteReader) throws(MalformedFrame) -> TerminalRow {
        try reader.need(rowFixedBytes)
        let index = try reader.u16()
        let flags = try reader.u8()
        let count = try reader.u16()
        var runs: [TerminalRun] = []
        runs.reserveCapacity(count)
        for _ in 0..<count {
            runs.append(try run(&reader))
        }
        return TerminalRow(index: index, wrapped: flags & rowWrapped != 0, runs: runs)
    }

    private static func run(_ reader: inout ByteReader) throws(MalformedFrame) -> TerminalRun {
        try reader.need(runFixedBytes)
        let column = try reader.u16()
        let span = try reader.u16()
        let kind = try reader.u8()
        let underline = try reader.u8()
        let attributes = try reader.number(UInt16.self)
        let colors = (foreground: try reader.u32(), background: try reader.u32(), underline: try reader.u32())
        let length = Int(try reader.u32())
        return TerminalRun(
            column: column,
            span: span,
            kind: kind,
            underline: underline,
            attributes: attributes,
            colors: colors,
            text: try reader.text(length)
        )
    }
}
