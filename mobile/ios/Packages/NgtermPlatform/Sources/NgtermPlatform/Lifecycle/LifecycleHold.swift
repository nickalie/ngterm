/// What the app holds on to that the background would cost it.
public enum LifecycleHold: Sendable {
    /// Nothing: the connections can go at once.
    case nothing
    /// Attached sessions, whose questions can still reach the user for a short grace.
    case sessions
    /// A machine waiting for the user to sign in: its connect must still be there when they have.
    case signIn
}
