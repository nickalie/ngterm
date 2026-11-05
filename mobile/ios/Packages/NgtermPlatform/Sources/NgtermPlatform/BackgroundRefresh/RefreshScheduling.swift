/// Asks for the next background refresh, or calls it off: the system's (``BackgroundRefresh``), or a test's record.
@preconcurrency
@MainActor
public protocol RefreshScheduling: AnyObject {
    /// Asks the system for a refresh some time from now, replacing one asked for before.
    func schedule()

    /// Calls off the refresh asked for, if any.
    func cancel()
}
