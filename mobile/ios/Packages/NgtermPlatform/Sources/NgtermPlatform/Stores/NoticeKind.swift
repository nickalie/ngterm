/// The kinds of notification the user can turn off (mobile/android's `AlertKind`), each on until then.
///
/// Stored by its case's name.
public enum NoticeKind: StoredChoice {
    /// An agent finished (status `completed`).
    case done
    /// A machine with attached sessions was lost for good: it needs the user to connect it again.
    case errors
    /// A notification a session's agent sent (`ngtermctl notify`).
    case notices
    /// An agent asks the user something (`ngtermctl ask`) while its session is out of sight.
    case questions
    /// An agent stopped to wait for the user (status `blocked`).
    case waiting
}
