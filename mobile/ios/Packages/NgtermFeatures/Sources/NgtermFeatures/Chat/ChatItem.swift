import NgtermCore

/// One row of a pane's chat, as the list shows it (mobile/android's `ChatItem`).
///
/// Its ``key`` names its event, so it stays the same through updates, older pages and a subagent's row becoming its
/// task's entry; the divider has its own.
public enum ChatItem: Equatable, Sendable, Identifiable {
    /// The pane's conversation switched here: what was shown before it is gone, and what follows is the new one.
    case divider
    /// An event of the conversation, its `body` as the latest event with its `id` left it (a tool's result completes
    /// its call), with the events of the subagent it started under it (`children`, for a task tool).
    case entry(id: String, body: ChatBody, children: [Self])
    /// A subagent's events whose task is not in the chat (its call is older than what was read), under the task's id
    /// (`parent`) until the task comes, which then takes them and this row's key.
    case subagent(parent: String, children: [Self])

    /// The row's key, the same whether its body is read (an entry) or only its subagent's events are (a subagent).
    public var key: String {
        switch self {
        case .entry(let id, _, _):
            Self.rowKey(id)

        case .subagent(let parent, _):
            Self.rowKey(parent)

        case .divider:
            "divider"
        }
    }

    /// The row's identity in the list: its ``key``.
    public var id: String {
        key
    }

    /// The subagent's steps under it: a task's, or a subagent row's.
    public var steps: [Self] {
        switch self {
        case .entry(_, _, let children), .subagent(_, let children):
            children

        case .divider:
            []
        }
    }

    /// The row's body, for an entry.
    public var body: ChatBody? {
        if case .entry(_, let body, _) = self {
            return body
        }
        return nil
    }

    /// The key of the row an event's `id` makes.
    public static func rowKey(_ id: String) -> String {
        "e:\(id)"
    }
}
