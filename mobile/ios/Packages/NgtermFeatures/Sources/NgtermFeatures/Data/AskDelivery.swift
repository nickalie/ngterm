import NgtermCore

/// How an ask is answered from its notification (mobile/android's `AskDelivery`, the ask rule).
public enum AskDelivery: Hashable, Sendable {
    /// A plain button ask: an action per button.
    case actions
    /// A choice ask, or one with more buttons than a notification holds: the notification opens the app.
    case openApp
    /// An input ask: a reply per answering button, and the cancel button as an action.
    case reply

    /// The most actions a notification shows; an ask with more buttons is answered in the app.
    public static let maxActions = 3

    /// How `ask` is answered from a notification.
    public init(_ ask: RemoteAsk) {
        if ask.choices != nil || ask.buttons.isEmpty || ask.buttons.count > Self.maxActions {
            self = .openApp
        } else if ask.input != nil {
            self = .reply
        } else {
            self = .actions
        }
    }
}
