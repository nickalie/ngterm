/// Where the app's notifications go: the system's (``SystemNotices``), or a test's record.
@preconcurrency
@MainActor
public protocol NoticeCenter: AnyObject {
    /// Posts `notice`, replacing one with its id.
    func post(_ notice: Notice)

    /// Takes down the notices `ids` that are up.
    func withdraw(_ ids: [String])

    /// Whether the user has answered the question about notifying: ``NoticePermission/undecided`` until then.
    func permission() async -> NoticePermission

    /// Asks the user, once, whether the app may notify; true when it may.
    func requestPermission() async -> Bool
}
