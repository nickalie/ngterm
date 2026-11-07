#if NGTERM_FAKES
    import NgtermCore

    /// One chat action a ``FakePane`` took, with its arguments.
    public enum FakeAction: Equatable, Sendable {
        /// ``FakePane/answerApproval(approval:label:)``.
        case approval(ChatApproval, label: String)
        /// ``FakePane/interrupt()``.
        case interrupt
        /// ``FakePane/sendPrompt(text:)``.
        case prompt(String)
        /// ``FakePane/answerQuestion(question:answers:)``.
        case question(ChatQuestion, answers: [QuestionAnswer])
    }
#endif
