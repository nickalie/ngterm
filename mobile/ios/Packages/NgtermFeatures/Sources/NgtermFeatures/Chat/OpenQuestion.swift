import NgtermCore

/// An `AskUserQuestion` still waiting for its answer: the chat event's `id` and the `question`.
public struct OpenQuestion: Equatable, Sendable {
    /// The chat event's id.
    public let id: String
    /// The questions.
    public let question: ChatQuestion

    /// The question `question` of the event `id`.
    public init(id: String, question: ChatQuestion) {
        self.id = id
        self.question = question
    }
}
