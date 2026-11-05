/// The cursor's shape, as the header's cursor style byte has it (0 block, 1 bar, 2 underline, 3 hollow block).
public enum CursorShape: UInt8, Sendable {
    /// A bar at the cell's left edge.
    case bar = 1
    /// A filled cell, the character under it in the cell's ground color.
    case block = 0
    /// The cell's outline: also how an unfocused terminal shows its cursor.
    case hollowBlock = 3
    /// A line along the cell's bottom.
    case underline = 2

    /// The shape the style byte `style` names; a block for one this build doesn't know.
    public init(style: UInt8) {
        self = Self(rawValue: style) ?? .block
    }
}
