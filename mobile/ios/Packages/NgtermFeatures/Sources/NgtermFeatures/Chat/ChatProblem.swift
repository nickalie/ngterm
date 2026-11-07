import NgtermCore

/// Why the last chat action did nothing, or not all it meant to.
public enum ChatProblem: Equatable, Sendable {
    /// The pane isn't shown any more (hidden, or its link went): nothing was sent.
    case closed
    /// A dialog is open, so a prompt or Stop would answer it: nothing was sent.
    case dialogOpen
    /// Another device kept control of the pane: nothing was sent.
    case leadRefused
    /// The screen doesn't show what the action answers, or not as the one it answers: nothing was sent.
    ///
    /// It was answered elsewhere, another call's dialog is up, or the screen can't be read.
    case notOnScreen
    /// The program isn't reading its input: nothing was sent; try again shortly.
    case notReading
    /// The core refused the action, for `reason` (its words): nothing was sent.
    case refused(reason: String)
    /// Keys went, but the screen didn't show what they should have done.
    case unconfirmed

    /// Whether the terminal is the way on: the banner offers Answer in terminal.
    public var offersTerminal: Bool {
        self == .notOnScreen || self == .unconfirmed
    }

    /// Whether the user can act on it (the attention tone), rather than it only reporting (the failed one).
    public var isActionable: Bool {
        self == .notOnScreen || self == .dialogOpen || self == .unconfirmed
    }

    /// What became of an action the core answered: none when it was sent.
    public init?(_ outcome: AgentInputOutcome) {
        switch outcome {
        case .sent:
            return nil

        case .notOnScreen:
            self = .notOnScreen

        case .dialogOpen:
            self = .dialogOpen

        case .leadRefused:
            self = .leadRefused

        case .unconfirmed:
            self = .unconfirmed
        }
    }

    /// Why the core refused an action.
    public init(_ error: PaneInputError) {
        switch error {
        case .Closed:
            self = .closed

        case .Backpressure:
            self = .notReading

        case .Refused(let reason):
            self = .refused(reason: reason)
        }
    }
}
