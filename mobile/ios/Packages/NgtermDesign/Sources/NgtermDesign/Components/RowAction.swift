/// The one action a state calls for: Connect, Retry now, Review, Add machine.
public struct RowAction {
    /// The action's words.
    public let title: String
    /// What it does.
    @preconcurrency public let perform: @MainActor () -> Void

    /// The action `title` doing `perform`.
    @preconcurrency
    public init(_ title: String, perform: @escaping @MainActor () -> Void) {
        self.title = title
        self.perform = perform
    }
}
