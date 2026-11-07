/// Whether a session's panes running an agent show as a chat, and how to set it for that session alone.
public struct ChatViewChoice {
    /// Whether they show as a chat: the session's own choice, else Settings' Agent chat view.
    public let isOn: Bool
    /// Sets the session's own choice; one equal to Settings' follows Settings again.
    public let set: @MainActor (Bool) -> Void

    /// The choice `isOn`, set through `set`.
    @preconcurrency
    public init(isOn: Bool, set: @escaping @MainActor (Bool) -> Void) {
        self.isOn = isOn
        self.set = set
    }
}
