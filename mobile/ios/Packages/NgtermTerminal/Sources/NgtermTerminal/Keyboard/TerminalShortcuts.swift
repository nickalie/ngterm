/// What the app does for the chords that move between panes and sessions (``TerminalShortcut``).
///
/// The session view sets them. A move with no action here is the program's chord, as any other key.
@preconcurrency
@MainActor
public struct TerminalShortcuts {
    /// ⌘⌥→.
    public var nextPane: (@MainActor () -> Void)?
    /// ⌘⌥←.
    public var previousPane: (@MainActor () -> Void)?
    /// ⌘⌥↓.
    public var nextSession: (@MainActor () -> Void)?
    /// ⌘⌥↑.
    public var previousSession: (@MainActor () -> Void)?

    /// The moves, each `nil` for none.
    @preconcurrency
    public init(
        nextPane: (@MainActor () -> Void)? = nil,
        previousPane: (@MainActor () -> Void)? = nil,
        nextSession: (@MainActor () -> Void)? = nil,
        previousSession: (@MainActor () -> Void)? = nil
    ) {
        self.nextPane = nextPane
        self.previousPane = previousPane
        self.nextSession = nextSession
        self.previousSession = previousSession
    }

    /// The action of the move `shortcut`.
    ///
    /// It is `nil` for a move with none, and for the text size's, which the terminal does itself.
    @preconcurrency
    public func action(for shortcut: TerminalShortcut) -> (@MainActor () -> Void)? {
        switch shortcut {
        case .bigger, .smaller, .resetSize:
            nil

        case .nextPane:
            nextPane

        case .previousPane:
            previousPane

        case .nextSession:
            nextSession

        case .previousSession:
            previousSession
        }
    }
}
