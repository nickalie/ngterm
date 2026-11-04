/// Where a session is drawn, as a main window sees it: here, in the window of its own it has, or in another main
/// window.
public enum SessionPlacement: Equatable, Sendable {
    /// In the window that asked.
    case here
    /// In another main window, which took it over: this window shows it again by taking it back, or once every
    /// window that took it over after this one lets it go.
    case otherWindow
    /// In its own window, which is brought forward: two views of one pane would fight over its size.
    case ownWindow
}
