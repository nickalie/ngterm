/// A run of cells of one style, left to right: the row buffer's run record (crates/ngterm-mobile/src/rows.rs).
///
/// Colors are `0xAARRGGBB`. A `0` in ``foreground`` or ``background`` is the terminal's default, resolved from the
/// header of the frame being drawn (``TerminalHeader/defaultForeground``, ``TerminalHeader/defaultBackground``), never
/// cached; a `0` ``underlineColor`` is the foreground.
public struct TerminalRun: Hashable, Sendable {
    /// What a run's cells carry besides their colors: the `CellFlags` of `ngterm-term`.
    public struct Attributes: OptionSet, Hashable, Sendable {
        /// Bold: the bold face.
        public static let bold = Self(rawValue: 1 << 0)
        /// Italic: the italic face.
        public static let italic = Self(rawValue: 1 << 1)
        /// Faint: the text at half its alpha.
        public static let faint = Self(rawValue: 1 << 2)
        /// Blinking.
        public static let blink = Self(rawValue: 1 << 3)
        /// Inverse: the text and the ground swap colors.
        public static let inverse = Self(rawValue: 1 << 4)
        /// Invisible: the ground with no text or lines on it.
        public static let invisible = Self(rawValue: 1 << 5)
        /// Struck through.
        public static let strikethrough = Self(rawValue: 1 << 6)
        /// A line over the cells.
        public static let overline = Self(rawValue: 1 << 7)
        /// Part of a hyperlink.
        public static let hyperlink = Self(rawValue: 1 << 8)

        /// The bits, as the buffer packs them.
        public let rawValue: UInt16

        /// The attributes with the bits `rawValue`.
        public init(rawValue: UInt16) {
            self.rawValue = rawValue
        }
    }

    /// The line under a run's cells, as the buffer's underline byte has it (0 is none).
    public enum Underline: UInt8, Sendable {
        /// A wave.
        case curly = 3
        /// Dashes.
        case dashed = 5
        /// Dots.
        case dotted = 4
        /// Two straight lines.
        case double = 2
        /// One straight line.
        case single = 1
    }

    /// Run kind: each glyph covers two columns.
    public static let wideKind: UInt8 = 1 << 0
    /// Run kind: the text is one grapheme cluster, one glyph at ``column`` over ``span`` columns.
    public static let clusterKind: UInt8 = 1 << 1

    /// The first column.
    public let column: Int
    /// The columns covered.
    public let span: Int
    /// The run kind bits (``wideKind``, ``clusterKind``).
    public let kind: UInt8
    /// The line under the cells; `nil` for none.
    public let underline: Underline?
    /// The attributes.
    public let attributes: Attributes
    /// The text's color (`0`: the default).
    public let foreground: UInt32
    /// The ground's color (`0`: the default).
    public let background: UInt32
    /// The underline's color (`0`: the foreground).
    public let underlineColor: UInt32
    /// The text: a code point per glyph, or one grapheme cluster.
    public let text: String

    /// Whether each glyph covers two columns.
    public var isWide: Bool {
        kind & Self.wideKind != 0
    }

    /// Whether the text is one grapheme cluster: one glyph at ``column``, ``span`` columns wide.
    public var isCluster: Bool {
        kind & Self.clusterKind != 0
    }

    /// The columns after the run's last one.
    public var end: Int {
        column + span
    }

    /// A run as the buffer packs it: `underline` 0 is none, and one this build doesn't know is a single line.
    public init(
        column: Int,
        span: Int,
        kind: UInt8,
        underline: UInt8,
        attributes: UInt16,
        colors: (foreground: UInt32, background: UInt32, underline: UInt32),
        text: String
    ) {
        self.column = column
        self.span = span
        self.kind = kind
        self.underline = underline == 0 ? nil : Underline(rawValue: underline) ?? .single
        self.attributes = Attributes(rawValue: attributes)
        foreground = colors.foreground
        background = colors.background
        underlineColor = colors.underline
        self.text = text
    }

    /// Whether the run covers `column`.
    public func covers(_ column: Int) -> Bool {
        (self.column..<end).contains(column)
    }
}
