/// An action a ``Notice`` offers: a button, or a reply typed beside it.
///
/// Answering is no step to take on a locked device: an action asks for the device to be unlocked first.
public struct NoticeAction: Hashable, Sendable {
    /// Its name, which the response carries back (``NoticeResponse/action``).
    public var id: String
    /// What its button says.
    public var title: String
    /// Whether it does what can't be undone: the system draws it so.
    public var isDestructive: Bool
    /// The hint in its reply field, when it takes typed text; `nil` for a plain button.
    public var replyPlaceholder: String?

    /// The action `id`, titled `title`, destructive when `isDestructive`, taking typed text with the hint
    /// `replyPlaceholder` when given.
    public init(id: String, title: String, isDestructive: Bool = false, replyPlaceholder: String? = nil) {
        self.id = id
        self.title = title
        self.isDestructive = isDestructive
        self.replyPlaceholder = replyPlaceholder
    }
}
