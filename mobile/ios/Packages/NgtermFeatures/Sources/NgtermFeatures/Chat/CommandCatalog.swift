import NgtermCore

/// The agent's slash commands and skills as the `/` menu has them, for the conversation on screen.
public enum CommandCatalog: Equatable, Sendable {
    /// The origin gave none (it predates the catalog, or the link went): the menu asks again as it next opens.
    case failed
    /// On its way from the origin.
    case fetching
    /// What the origin listed.
    case ready([AgentCommand])
    /// Not asked for in this conversation yet: the menu asks as it opens.
    case unasked
}
