import NgtermCore

/// The account the agent is signed in with and its weekly limit, as far as the origin reads them.
public struct ChatAccount: Equatable, Sendable {
    /// Who is signed in.
    public let email: String?
    /// How much of the weekly limit is used, and when it resets.
    public let weekly: ChatUsage?
}
