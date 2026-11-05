/// What a session view shows of its session (mobile/android's `OpenSession`).
public enum OpenSession: Equatable, Sendable {
    /// Ended on its machine while it was open, last seen as `name` on `machine`.
    case gone(name: String, machine: String)
    /// Attached: `session`, with the pane selected (`focused`; `nil` before its layout arrives), the one every key goes
    /// to.
    case live(AttachedSession, focused: String?)
}
