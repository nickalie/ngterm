/// The New session sheet while it is open: whether the create is running, and why the last one failed.
public struct NewSessionState: Hashable, Sendable {
    /// Whether a create is under way.
    public var isRunning: Bool
    /// Why the last create failed (the core's words), until another starts.
    public var failure: String?

    /// A sheet with a create `isRunning` or not, and the last one's `failure`.
    public init(isRunning: Bool = false, failure: String? = nil) {
        self.isRunning = isRunning
        self.failure = failure
    }
}
