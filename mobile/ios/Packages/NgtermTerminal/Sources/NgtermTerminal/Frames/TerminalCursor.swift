/// Where the cursor is and how it is drawn: there is one to draw when it is both in view and visible (``shown``).
public struct TerminalCursor: Hashable, Sendable {
    /// No cursor, before the first frame.
    public static let hidden = Self(column: 0, row: 0, shape: .block, flags: 0)

    private static let inViewFlag: UInt8 = 1 << 0
    private static let visibleFlag: UInt8 = 1 << 1
    private static let blinkingFlag: UInt8 = 1 << 2
    private static let passwordFlag: UInt8 = 1 << 3
    private static let wideTailFlag: UInt8 = 1 << 4

    /// Its column.
    public let column: Int
    /// Its row.
    public let row: Int
    /// Its shape.
    public let shape: CursorShape
    /// Whether its position is on screen.
    public let inView: Bool
    /// Whether the program shows it (DECTCEM).
    public let visible: Bool
    /// Whether it blinks.
    public let blinking: Bool
    /// Whether it is at a password prompt.
    public let password: Bool
    /// Whether it is on a wide character's second column.
    public let wideTail: Bool

    /// Whether there is a cursor to draw.
    public var shown: Bool {
        inView && visible
    }

    /// The column it is drawn from: a wide character's first, when it is on its second.
    public var drawnColumn: Int {
        wideTail ? max(column - 1, 0) : column
    }

    /// The cursor at `column`, `row` in `shape`, with the header's cursor `flags`.
    public init(column: Int, row: Int, shape: CursorShape, flags: UInt8) {
        self.column = column
        self.row = row
        self.shape = shape
        inView = flags & Self.inViewFlag != 0
        visible = flags & Self.visibleFlag != 0
        blinking = flags & Self.blinkingFlag != 0
        password = flags & Self.passwordFlag != 0
        wideTail = flags & Self.wideTailFlag != 0
    }
}
