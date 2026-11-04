/// Where a session is drawn, as a main window sees it: here, in the window of its own it has, or in another main
/// window.
public enum SessionPlacement: Equatable, Sendable {
    /// In the window that asked.
    case here
    /// In another main window, which took it over: this window shows it again only by taking it back.
    case otherWindow
    /// In its own window, which is brought forward: two views of one pane would fight over its size.
    case ownWindow
}
