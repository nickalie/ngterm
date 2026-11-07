/// The conversation an origin names for a pane's chat: its agent, its id, its working directory and model.
public struct ChatConversation: Equatable, Sendable {
    /// The agent's wire name: `claude`, `codex`, or one a newer origin knows.
    public let agent: String
    /// The conversation's id.
    public let id: String
    /// The agent's working directory, when known.
    public let cwd: String?
    /// The model, when known.
    public let model: String?
}
