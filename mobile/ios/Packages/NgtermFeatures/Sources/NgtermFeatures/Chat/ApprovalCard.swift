import NgtermCore

/// The permission prompt the agent waits on: the chat event's `id`, the `approval` its hook reported, and the buttons'
/// `labels`.
///
/// The labels are the dialog's own options, read from the pane's screen, or the agent's set when the screen can't be
/// read; empty until read.
public struct ApprovalCard: Equatable, Sendable {
    /// The chat event's id.
    public let id: String
    /// The prompt, as the agent's hook reported it.
    public let approval: ChatApproval
    /// The dialog's options, in its order; empty until read.
    public var labels: [String]

    /// The card for `approval`, the event `id`, offering `labels`.
    public init(id: String, approval: ChatApproval, labels: [String]) {
        self.id = id
        self.approval = approval
        self.labels = labels
    }
}
