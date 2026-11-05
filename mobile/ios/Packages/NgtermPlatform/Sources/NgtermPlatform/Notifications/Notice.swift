/// A local notification the app posts: what it says, the thread it groups under, what its tap carries back, and the
/// actions it offers (none: a tap opens the app).
///
/// A later notice with the same ``id`` replaces it, and withdrawing the id takes it down.
public struct Notice: Hashable, Sendable {
    /// Its name: a later notice with it replaces this one.
    public var id: String
    /// The thread it groups under (a machine and session's notices together).
    public var threadId: String
    /// Its title.
    public var title: String
    /// The line under the title; empty for none.
    public var subtitle: String
    /// Its text.
    public var body: String
    /// What a response to it carries back to the app (``NoticeResponse/info``): where its tap leads, which ask it is.
    public var info: [String: String]
    /// Its actions, in order; empty for none.
    public var actions: [NoticeAction]

    /// The notice `id` in the thread `threadId`, saying `title`, `body` and `subtitle`, carrying `info` and offering
    /// `actions`.
    public init(
        id: String,
        threadId: String,
        title: String,
        body: String,
        subtitle: String = "",
        info: [String: String] = [:],
        actions: [NoticeAction] = []
    ) {
        self.id = id
        self.threadId = threadId
        self.title = title
        self.subtitle = subtitle
        self.body = body
        self.info = info
        self.actions = actions
    }
}
