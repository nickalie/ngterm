/// Whether the app may notify, as the user has answered.
public enum NoticePermission: Sendable {
    /// The app may notify.
    case allowed
    /// The user said no; Settings is the way back.
    case denied
    /// The user hasn't been asked yet.
    case undecided
}
