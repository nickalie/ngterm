/// Where a session opens: here, in the window that asked, or in the window of its own it already has.
public enum SessionPlacement: Equatable, Sendable {
    /// In the window that asked.
    case here
    /// In its own window, which is brought forward: two views of one pane would fight over its size.
    case ownWindow
}
