#if NGTERM_FAKES
    /// A run of a ``RowBufferWriter`` row: its text from its first column, over ``span`` columns.
    ///
    /// Colors are `0xAARRGGBB`, `0` for the terminal's default; ``kind``, ``underline`` and ``attributes`` are the
    /// layout's bits (crates/ngterm-mobile/src/rows.rs).
    public struct FakeRun: Equatable, Sendable {
        /// Run kind: each glyph covers two columns.
        public static let wide: UInt8 = 1
        /// Run kind: the text is one grapheme cluster, one glyph.
        public static let cluster: UInt8 = 2

        /// Attribute: bold.
        public static let bold: UInt16 = 1
        /// Attribute: italic.
        public static let italic: UInt16 = 2
        /// Attribute: faint.
        public static let faint: UInt16 = 4
        /// Attribute: inverse.
        public static let inverse: UInt16 = 16
        /// Attribute: invisible.
        public static let invisible: UInt16 = 32
        /// Attribute: struck through.
        public static let strikethrough: UInt16 = 64
        /// Attribute: overlined.
        public static let overline: UInt16 = 128

        private static let wideColumns = 2

        /// The first column.
        public let column: Int
        /// The text.
        public let text: String
        /// The run kind bits (``wide``, ``cluster``).
        public let kind: UInt8
        /// The columns covered.
        public let span: Int
        /// The underline: 0 none, 1 single, 2 double, 3 curly, 4 dotted, 5 dashed.
        public let underline: UInt8
        /// The attribute bits (``bold`` and the rest).
        public let attributes: UInt16
        /// The text's color.
        public let foreground: UInt32
        /// The ground's color.
        public let background: UInt32
        /// The underline's color (`0`: the foreground).
        public let underlineColor: UInt32

        /// A run of `text` from `column`, covering `span` columns: by default one per code point, or one glyph for a
        /// cluster, two columns each in a wide run.
        public init(
            _ column: Int,
            _ text: String,
            kind: UInt8 = 0,
            span: Int? = nil,
            underline: UInt8 = 0,
            attributes: UInt16 = 0,
            foreground: UInt32 = 0,
            background: UInt32 = 0,
            underlineColor: UInt32 = 0
        ) {
            let glyphs = kind & Self.cluster != 0 ? 1 : text.unicodeScalars.count
            self.column = column
            self.text = text
            self.kind = kind
            self.span = span ?? (kind & Self.wide != 0 ? glyphs * Self.wideColumns : glyphs)
            self.underline = underline
            self.attributes = attributes
            self.foreground = foreground
            self.background = background
            self.underlineColor = underlineColor
        }
    }
#endif
