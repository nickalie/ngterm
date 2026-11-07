import NgtermCore

/// The `/` menu over the composer, for what follows the `/` (`query`).
public enum SlashMenu: Equatable, Sendable {
    /// The `commands` matching `query`, best first (none: nothing matches); Return or Tab picks the `highlighted`.
    case commands(query: String, commands: [AgentCommand], highlighted: Int)
    /// No commands to list: the catalog is on its way (`isFetching`), or the origin listed none.
    ///
    /// The origin predates the catalog, the link went, or the agent has none. Either is a quiet line, never an error:
    /// the prompt still goes as typed.
    case waiting(query: String, isFetching: Bool)

    /// What follows the `/`.
    public var query: String {
        switch self {
        case .waiting(let query, _), .commands(let query, _, _):
            query
        }
    }
}
