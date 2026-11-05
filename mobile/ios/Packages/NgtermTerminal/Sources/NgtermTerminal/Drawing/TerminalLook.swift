/// How the view wants its rows drawn: the palette's selection color, and whether it has the focus (an unfocused
/// terminal shows its cursor as a hollow block).
public struct TerminalLook: Hashable, Sendable {
    /// The selection's fill, `0xAARRGGBB`, under text that keeps its own colors.
    public let selection: UInt32
    /// Whether the terminal has the focus.
    public let focused: Bool

    /// A look with the selection in `selection` and the focus `focused`.
    public init(selection: UInt32, focused: Bool) {
        self.selection = selection
        self.focused = focused
    }
}
