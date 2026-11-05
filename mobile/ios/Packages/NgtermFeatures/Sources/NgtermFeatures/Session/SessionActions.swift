/// What a session view asks of the window it is in: to close it, to open another session (the keyboard's moves and
/// the sidebar), and, on an iPad, to give the session a window of its own.
@preconcurrency
@MainActor
public struct SessionActions {
    /// Leaves the session: back to the list, or the session's own window closes.
    public let close: @MainActor () -> Void
    /// Opens another attached session in this window (or brings forward its own).
    public let open: @MainActor (SessionKey) -> Void
    /// Opens the session in a window of its own, or brings that window forward; `nil` where there is none to have.
    public let openInNewWindow: (@MainActor () -> Void)?

    /// The actions `close`, `open` and `openInNewWindow`.
    @preconcurrency
    public init(
        close: @escaping @MainActor () -> Void,
        open: @escaping @MainActor (SessionKey) -> Void,
        openInNewWindow: (@MainActor () -> Void)?
    ) {
        self.close = close
        self.open = open
        self.openInNewWindow = openInNewWindow
    }
}
