/// How a pane running an agent shows: its chat, or its terminal.
public enum PaneView: Equatable, Sendable {
    /// Its chat.
    case chat
    /// Its terminal.
    case terminal
}
