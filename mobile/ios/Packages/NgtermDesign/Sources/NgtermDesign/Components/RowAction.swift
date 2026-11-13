/// The one action a state calls for: Connect, Retry now, Review, Add machine.
public struct RowAction {
    /// The action's words.
    public let title: String
    /// What it does.
    @preconcurrency public let perform: @MainActor () -> Void
    /// Whether it takes a tap now; otherwise every button for it shows disabled and VoiceOver isn't offered it.
    public let isEnabled: Bool

    /// The action `title` doing `perform`, taking a tap while `isEnabled`.
    @preconcurrency
    public init(_ title: String, isEnabled: Bool = true, perform: @escaping @MainActor () -> Void) {
        self.title = title
        self.isEnabled = isEnabled
        self.perform = perform
    }
}
