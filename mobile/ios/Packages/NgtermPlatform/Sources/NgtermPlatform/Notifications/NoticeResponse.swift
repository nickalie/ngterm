/// The user's response to a ``Notice``: its tap, or one of its actions with what was typed.
public struct NoticeResponse: Hashable, Sendable {
    /// The notice's ``Notice/id``.
    public var noticeId: String
    /// The action's ``NoticeAction/id``; `nil` for a tap on the notice itself.
    public var action: String?
    /// What was typed in the action's reply field; `nil` for none.
    public var text: String?
    /// What the notice carried (``Notice/info``).
    public var info: [String: String]

    /// The response to the notice `noticeId` carrying `info`: its `action` (`nil` for a tap), with `text` typed.
    public init(noticeId: String, action: String?, info: [String: String], text: String? = nil) {
        self.noticeId = noticeId
        self.action = action
        self.text = text
        self.info = info
    }
}
