/// The agent a pane runs, as its origin announces it (mobile/android's `PaneAgent`): `agent` (`claude`, `codex`, or a
/// name a newer origin knows) and its `conversation` (`nil` until the origin knows it).
public struct PaneAgent: Hashable, Sendable {
    /// The agent's wire name.
    public let agent: String
    /// The conversation it is in, once the origin knows it.
    public let conversation: String?

    /// `agent` in `conversation`.
    public init(agent: String, conversation: String?) {
        self.agent = agent
        self.conversation = conversation
    }
}
