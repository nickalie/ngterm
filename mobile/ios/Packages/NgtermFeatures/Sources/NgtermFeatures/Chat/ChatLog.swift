import NgtermCore

/// A pane's chat folded into its rows (mobile/android's `ChatLog`), a value: ``appending(_:)`` takes the events the
/// core streams, ``prepending(_:)`` an older page, and ``reset()`` starts over after the conversation switched
/// (``switched`` puts a divider on top).
///
/// - Events upsert by id: a later event with an id already shown updates its row in place, the newest word winning. A
///   tool result whose call was outside the origin's window comes with no name or title, and keeps the ones its call
///   gave.
/// - An older page is folded on its own first, oldest to newest (the origin repeats an id there as it changed: a
///   tool's call, then its result), then goes in beneath the log: an id the log already holds keeps its place and what
///   the log's newer events said, the page filling in only what they left out.
/// - An event with a `parent` (a subagent's) folds under the row with that id (its task tool), in the order read;
///   while that row isn't here, under a subagent row in its place. The task, once it comes, takes the placeholder's
///   events and key: live in the placeholder's place, from an older page at its own place in the page.
/// - Only what the list shows is kept: prompts, replies, thinking, tools, questions and notices. The conversation's own
///   record, approvals, `live`, `reset` and entries the origin couldn't read are the chat model's or nobody's.
public struct ChatLog: Equatable, Sendable {
    // a log's parts, copied to change
    private struct Parts: Equatable, Sendable {
        // the top-level ids, oldest first, a missing task's id among them
        var order: [String] = []
        // every kept event's latest body, by id
        var bodies: [String: ChatBody] = [:]
        // each parent's events, oldest first
        var children: [String: [String]] = [:]
        // each subagent event's parent, and those events in the order they were hung
        var parents: [String: String] = [:]
        var parentOrder: [String] = []

        // a live event: it updates its row, fills its placeholder in place, or goes in last
        mutating func newer(_ event: ChatEvent) {
            let id = event.id
            if let known = bodies[id] {
                bodies[id] = ChatLog.merge(older: known, newer: event.body)
                return
            }
            bodies[id] = event.body
            guard let parent = event.parent else {
                if !order.contains(id) {
                    order.append(id)
                }
                return
            }
            // a placeholder whose task is itself a subagent's goes under that subagent's task
            order.removeAll { $0 == id }
            hang(id, under: parent)
        }

        private mutating func hang(_ id: String, under parent: String) {
            if parents.updateValue(parent, forKey: id) == nil {
                parentOrder.append(id)
            }
            children[parent, default: []].append(id)
            // a parent with no body (every subagent event has one) stands in for its task
            if bodies[parent] == nil, !order.contains(parent) {
                order.append(parent)
            }
        }
    }

    // an older page going in, folded by id: the rows it puts on top (front) and under each parent (kids), oldest first
    private struct Page {
        var parts: Parts
        var front: [String] = []
        var kids: [String: [String]] = [:]
        // the parents in the order the page first hung a step under each
        var kidOrder: [String] = []

        init(_ parts: Parts) {
            self.parts = parts
        }

        mutating func take(_ event: ChatEvent) {
            let id = event.id
            if let known = parts.bodies[id] {
                // the log's word is newer: the page fills in only what it left out, and the row keeps its place
                parts.bodies[id] = ChatLog.merge(older: event.body, newer: known)
                return
            }
            parts.bodies[id] = event.body
            // a task its subagent's events stood in for (in the log, or earlier in the page) takes them, at its own
            // place
            parts.order.removeAll { $0 == id }
            front.removeAll { $0 == id }
            if let parent = event.parent {
                under(id, parent)
            } else {
                top(id)
            }
        }

        private mutating func top(_ id: String) {
            if !front.contains(id) {
                front.append(id)
            }
        }

        private mutating func under(_ id: String, _ parent: String) {
            if parts.parents.updateValue(parent, forKey: id) == nil {
                parts.parentOrder.append(id)
            }
            if kids[parent] == nil {
                kidOrder.append(parent)
            }
            kids[parent, default: []].append(id)
            if parts.bodies[parent] == nil, !parts.order.contains(parent) {
                top(parent)
            }
        }

        mutating func done() -> Parts {
            for parent in kidOrder {
                parts.children[parent] = (kids[parent] ?? []) + (parts.children[parent] ?? [])
            }
            parts.order.insert(contentsOf: front, at: 0)
            return parts
        }
    }

    /// The rows, oldest first.
    public let items: [ChatItem]
    /// The conversation's own tool calls with no result yet, oldest first.
    ///
    /// A command or a subagent still on its way, including one the agent left running in the background when its turn
    /// ended. A subagent's own steps count as its task's, so only rows on the conversation's level are here.
    public let running: [ChatTool]
    /// Whether the conversation switched since the chat opened: a divider heads the rows.
    public let switched: Bool
    private let parts: Parts

    /// Whether nothing is shown.
    public var isEmpty: Bool {
        parts.order.isEmpty
    }

    /// An empty log.
    public init() {
        self.init(parts: Parts(), switched: false)
    }

    private init(parts: Parts, switched: Bool) {
        self.parts = parts
        self.switched = switched
        let rows = parts.order.map { Self.item(of: $0, parts) }
        items = switched ? [.divider] + rows : rows
        running = parts.order.compactMap { id in
            guard case .tool(let tool) = parts.bodies[id], tool.status == .running else {
                return nil
            }
            return tool
        }
    }

    // the newer word on an event wins; a tool result read without its call keeps the call's name, kind and title
    internal static func merge(older: ChatBody, newer: ChatBody) -> ChatBody {
        guard case .tool(let call) = older, case .tool(var result) = newer, result.name.isEmpty else {
            return newer
        }
        result.name = call.name
        result.kind = call.kind
        if result.title.isEmpty {
            result.title = call.title
        }
        return .tool(tool: result)
    }

    // what the list keeps of the events: what it shows, and no event its own subagent's
    private static func isKept(_ event: ChatEvent) -> Bool {
        guard event.parent != event.id else {
            return false
        }
        switch event.body {
        case .user, .assistant, .thinking, .tool, .question, .notice:
            return true

        default:
            return false
        }
    }

    // a page's events by id, in the order each id first came, each one's latest word folded over its earlier ones
    private static func folded(_ events: [ChatEvent]) -> [ChatEvent] {
        var ids: [String] = []
        var byId: [String: ChatEvent] = [:]
        for event in events {
            guard var before = byId[event.id] else {
                ids.append(event.id)
                byId[event.id] = event
                continue
            }
            before.body = merge(older: before.body, newer: event.body)
            before.parent = before.parent ?? event.parent
            before.cursor = event.cursor
            byId[event.id] = before
        }
        return ids.compactMap { byId[$0] }
    }

    private static func item(of id: String, _ parts: Parts) -> ChatItem {
        let kids = (parts.children[id] ?? []).map { item(of: $0, parts) }
        if let body = parts.bodies[id] {
            return .entry(id: id, body: body, children: kids)
        }
        return .subagent(parent: id, children: kids)
    }

    /// The latest `AskUserQuestion` still waiting for its answer, leaving out the `dismissed` ones.
    public func openQuestion(dismissed: Set<String>) -> OpenQuestion? {
        for id in (parts.order + parts.parentOrder).reversed() {
            guard case .question(let question) = parts.bodies[id], question.status == .running,
                !dismissed.contains(id)
            else {
                continue
            }
            return OpenQuestion(id: id, question: question)
        }
        return nil
    }

    /// The log with `events` (newest last) taken in after what it shows.
    public func appending(_ events: [ChatEvent]) -> Self {
        var next = parts
        for event in events where Self.isKept(event) {
            next.newer(event)
        }
        return Self(parts: next, switched: switched)
    }

    /// The log with an older page (`events`, oldest first) put before what it shows.
    public func prepending(_ events: [ChatEvent]) -> Self {
        var page = Page(parts)
        for event in Self.folded(events.filter(Self.isKept)) {
            page.take(event)
        }
        return Self(parts: page.done(), switched: switched)
    }

    /// An empty log after the conversation switched: the divider, and nothing before it.
    public func reset() -> Self {
        Self(parts: Parts(), switched: true)
    }
}
