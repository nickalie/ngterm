/// Where the app's notifications go: the system's (``SystemNotices``), or a test's record.
@preconcurrency
@MainActor
public protocol NoticeCenter: AnyObject {
    /// Posts `notice`, replacing one with its id.
    func post(_ notice: Notice)

    /// Takes down the notices `ids` that are up.
    func withdraw(_ ids: [String])

    /// Waits until every post and withdrawal made so far has reached the system: a background run awaits it before it
    /// ends, as the system may suspend the process once it has.
    func delivered() async

    /// Whether the user has answered the question about notifying: ``NoticePermission/undecided`` until then.
    func permission() async -> NoticePermission

    /// Asks the user, once, whether the app may notify; true when it may.
    func requestPermission() async -> Bool
}
