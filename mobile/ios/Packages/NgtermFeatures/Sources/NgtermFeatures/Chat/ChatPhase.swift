/// Where a pane's chat stands.
public enum ChatPhase: Equatable, Sendable {
    /// It ended for good (the agent ended, or the origin offers no chat): the pane shows its terminal.
    case ended
    /// Its backlog is read, and new events follow.
    case live
    /// Opened, its backlog on its way.
    case loading
    /// Its link went; it resumes on its own from where it stood.
    case reconnecting
}
