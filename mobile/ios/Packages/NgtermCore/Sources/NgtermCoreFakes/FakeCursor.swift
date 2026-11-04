#if NGTERM_FAKES
    /// The cursor of a ``RowBufferWriter`` frame.
    public struct FakeCursor: Equatable, Sendable {
        /// Its column.
        public let column: Int
        /// Its row.
        public let row: Int
        /// Its style: 0 block, 1 bar, 2 underline, 3 hollow block.
        public let style: UInt8
        /// Whether it is shown.
        public let visible: Bool
        /// Whether it blinks.
        public let blinking: Bool
        /// Whether it is on a wide character's second column.
        public let wideTail: Bool

        /// A cursor at `column`, `row`.
        public init(
            column: Int,
            row: Int,
            style: UInt8 = 0,
            visible: Bool = true,
            blinking: Bool = false,
            wideTail: Bool = false
        ) {
            self.column = column
            self.row = row
            self.style = style
            self.visible = visible
            self.blinking = blinking
            self.wideTail = wideTail
        }
    }
#endif
