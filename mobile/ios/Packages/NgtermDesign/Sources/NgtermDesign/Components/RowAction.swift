/// The one action a state calls for: Connect, Retry now, Review, Add machine.
public struct RowAction {
    /// The action's words.
    public let title: String
    /// What it does.
    @preconcurrency public let perform: @MainActor () -> Void
    /// Whether it takes a tap now; a button for it shows disabled otherwise.
    public let isEnabled: Bool

    /// The action `title` doing `perform`, taking a tap while `isEnabled`.
    @preconcurrency
    public init(_ title: String, isEnabled: Bool = true, perform: @escaping @MainActor () -> Void) {
        self.title = title
        self.isEnabled = isEnabled
        self.perform = perform
    }
}
