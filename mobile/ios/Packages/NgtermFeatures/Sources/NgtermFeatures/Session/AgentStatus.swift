/// An agent's status as its origin sets it: its wire name, its `#rrggbb` color and its shape's wire name.
public struct AgentStatus: Hashable, Sendable {
    /// The status's wire name ("working", "blocked", …).
    public let text: String
    /// The color its origin set, if any.
    public let color: String?
    /// The shape its origin set, if any.
    public let shape: String?

    /// The status `text`, in `color` and `shape` when its origin set them.
    public init(text: String, color: String? = nil, shape: String? = nil) {
        self.text = text
        self.color = color
        self.shape = shape
    }
}
