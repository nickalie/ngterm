#if NGTERM_FAKES
    import Foundation

    /// Packs a frame as the core's `Pane.frame` does (crates/ngterm-mobile/src/rows.rs, version 1).
    ///
    /// It is for fakes that hand a view frames with no native library behind them; its test holds it to the Rust
    /// writer's golden fixture. Each step hands back a writer with one more thing set.
    public struct RowBufferWriter: Sendable {
        /// Ghostty's default text color, the terminal's before the view gives it a palette.
        public static let ghosttyForeground: UInt32 = 0xFF_FFFFFF
        /// Ghostty's default ground color.
        public static let ghosttyBackground: UInt32 = 0xFF_282C34

        private static let magic = Data("NGRB".utf8)
        private static let version: UInt16 = 1
        private static let frameFull: UInt16 = 1
        private static let frameStale: UInt16 = 2
        private static let cursorInView: UInt8 = 1
        private static let cursorVisible: UInt8 = 2
        private static let cursorBlinking: UInt8 = 4
        private static let cursorWideTail: UInt8 = 16
        private static let rowWrapped: UInt8 = 1

        private let columns: Int
        private let rows: Int
        private let seq: UInt64
        private let full: Bool
        private let cursor: FakeCursor?
        private let stale: Bool
        private var foreground = Self.ghosttyForeground
        private var background = Self.ghosttyBackground
        private var cursorColor: UInt32 = 0
        private var records: [Int: (wrapped: Bool, runs: [FakeRun])] = [:]
        private var selection: [Int: (first: Int, last: Int)] = [:]

        private var cursorFlags: UInt8 {
            guard let cursor else {
                return 0
            }
            var flags = Self.cursorInView
            if cursor.visible {
                flags |= Self.cursorVisible
            }
            if cursor.blinking {
                flags |= Self.cursorBlinking
            }
            if cursor.wideTail {
                flags |= Self.cursorWideTail
            }
            return flags
        }

        /// A frame of `columns` × `rows` with the cursor `cursor` (none in view when `nil`), numbered `seq`; `full`
        /// when every row follows, `stale` when the terminal was busy.
        public init(
            columns: Int,
            rows: Int,
            seq: UInt64 = 1,
            full: Bool = true,
            cursor: FakeCursor? = FakeCursor(column: 0, row: 0),
            stale: Bool = false
        ) {
            self.columns = columns
            self.rows = rows
            self.seq = seq
            self.full = full
            self.cursor = cursor
            self.stale = stale
        }

        private static func put(_ value: some FixedWidthInteger, into data: inout Data) {
            withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) }
        }

        private static func put(_ value: Int, into data: inout Data) {
            put(UInt16(truncatingIfNeeded: value), into: &data)
        }

        private static func run(_ run: FakeRun, into data: inout Data) {
            let text = Data(run.text.utf8)
            put(run.column, into: &data)
            put(run.span, into: &data)
            put(run.kind, into: &data)
            put(run.underline, into: &data)
            put(run.attributes, into: &data)
            put(run.foreground, into: &data)
            put(run.background, into: &data)
            put(run.underlineColor, into: &data)
            put(UInt32(text.count), into: &data)
            data.append(text)
        }

        /// The default text and ground, and the cursor's color (`0`: it follows the cell); Ghostty's, none by default.
        public func colors(foreground: UInt32, background: UInt32, cursor: UInt32 = 0) -> Self {
            var writer = self
            writer.foreground = foreground
            writer.background = background
            writer.cursorColor = cursor
            return writer
        }

        /// Row `index` with `runs`, left to right; `wrapped` when its text continues on the next row.
        public func row(_ index: Int, _ runs: FakeRun..., wrapped: Bool = false) -> Self {
            var writer = self
            writer.records[index] = (wrapped, runs)
            return writer
        }

        /// Row `index` as one run of default-colored `text`.
        public func text(_ index: Int, _ text: String) -> Self {
            row(index, FakeRun(0, text))
        }

        /// Selects row `index` from column `first` to `last`, both included.
        public func select(_ index: Int, first: Int, last: Int) -> Self {
            var writer = self
            writer.selection[index] = (first, last)
            return writer
        }

        /// The packed frame.
        public func bytes() -> Data {
            var data = Self.magic
            Self.put(Self.version, into: &data)
            Self.put((full ? Self.frameFull : 0) | (stale ? Self.frameStale : 0), into: &data)
            Self.put(seq, into: &data)
            Self.put(columns, into: &data)
            Self.put(rows, into: &data)
            Self.put(foreground, into: &data)
            Self.put(background, into: &data)
            Self.put(cursorColor, into: &data)
            Self.put(cursor?.column ?? 0, into: &data)
            Self.put(cursor?.row ?? 0, into: &data)
            Self.put(cursor?.style ?? 0, into: &data)
            Self.put(cursorFlags, into: &data)
            Self.put(records.count, into: &data)
            for (index, record) in records.sorted(by: { $0.key < $1.key }) {
                Self.put(index, into: &data)
                Self.put(record.wrapped ? Self.rowWrapped : 0, into: &data)
                Self.put(record.runs.count, into: &data)
                for run in record.runs {
                    Self.run(run, into: &data)
                }
            }
            Self.put(selection.count, into: &data)
            for (index, span) in selection.sorted(by: { $0.key < $1.key }) {
                Self.put(index, into: &data)
                Self.put(span.first, into: &data)
                Self.put(span.last, into: &data)
            }
            return data
        }
    }
#endif
