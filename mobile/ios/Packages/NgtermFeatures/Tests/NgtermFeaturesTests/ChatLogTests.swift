import NgtermCore
import Testing

@testable import NgtermFeatures

/// How the chat's log merges live events, older pages, placeholders and repeated ids, and the rows' keys through it all
/// (mobile/android's `ChatLogTest`).
@Suite("the chat's log")
internal struct ChatLogTests {
    private typealias Samples = ChatSamples

    private let task = Samples.tool("Agent", "Find the bug", .running, kind: .task)

    // the rows as ids (a placeholder `?id`, the divider `--`), each with its children, each row's key its event's
    private static func rows(_ items: [ChatItem]) -> [String] {
        items.map { item in
            switch item {
            case .entry(let id, _, let children):
                #expect(item.key == "e:\(id)")
                return children.isEmpty ? id : "\(id)[\(rows(children).joined(separator: ", "))]"

            case .subagent(let parent, let children):
                #expect(item.key == "e:\(parent)")
                return "?\(parent)[\(rows(children).joined(separator: ", "))]"

            case .divider:
                return "--"
            }
        }
    }

    private static func flat(_ items: [ChatItem]) -> [ChatItem] {
        items.flatMap { [$0] + flat($0.steps) }
    }

    private static func body(_ log: ChatLog, _ id: String) -> ChatBody? {
        flat(log.items).first { $0.key == ChatItem.rowKey(id) }?.body
    }

    private static func tool(_ body: ChatBody?) -> ChatTool? {
        if case .tool(let tool) = body {
            return tool
        }
        return nil
    }

    @Test("running lists the conversation's calls with no result yet, not a subagent's steps")
    internal func running() {
        let log = ChatLog().appending([
            Samples.event("t1", Samples.tool("Bash", "npm test", .ok, output: nil)),
            Samples.event("task", task),
            Samples.event("s1", Samples.tool("Bash", "cargo test"), parent: "task"),
            Samples.event("t2", Samples.tool("Bash", "npm run build")),
        ])
        #expect(log.running.map(\.title) == ["Find the bug", "npm run build"])
        // a background task's notification finishes it
        let finished = Samples.event("task", Samples.tool("Agent", "Find the bug", .ok, kind: .task))
        let done = log.appending([finished])
        #expect(done.running.map(\.title) == ["npm run build"])
    }

    @Test("live events update their rows, newest last")
    internal func liveEvents() {
        let log = ChatLog()
            .appending([
                Samples.event("a1", Samples.reply("Let")),
                Samples.event("a1", Samples.reply("Let me")),
                Samples.event("u1", Samples.user("hi")),
            ])
            .appending([Samples.event("a1", Samples.reply("Let me look."))])
        #expect(Self.rows(log.items) == ["a1", "u1"])
        #expect(Self.body(log, "a1") == Samples.reply("Let me look."))
    }

    @Test("a task streamed after its subagent's events takes its placeholder, in place")
    internal func placeholder() {
        let before = ChatLog().appending([
            Samples.event("c1", Samples.reply("Found it"), parent: "task"),
            Samples.event("a1", Samples.reply("Waiting")),
            // a subagent whose task is itself one of the first subagent's events, not read yet
            Samples.event("n1", Samples.reply("Nested"), parent: "inner"),
        ])
        #expect(Self.rows(before.items) == ["?task[c1]", "a1", "?inner[n1]"])
        let keys = before.items.map(\.key)
        let after = before.appending([Samples.event("task", task), Samples.event("inner", task, parent: "task")])
        #expect(Self.rows(after.items) == ["task[c1, inner[n1]]", "a1"])
        // the task's row kept its key and its place; the nested one went under its parent with its key
        #expect(Array(after.items.map(\.key).prefix(2)) == Array(keys.prefix(2)))
        #expect(after.items.first?.steps.last?.key == "e:inner")
    }

    @Test("an older page is folded oldest to newest before it goes in")
    internal func olderPageFolded() throws {
        let log = ChatLog()
            .appending([Samples.event("u5", Samples.user("five"))])
            .prepending([
                Samples.event("t1", Samples.tool("Bash", "make")),
                Samples.event("q1", Samples.question()),
                Samples.event("t1", Samples.tool("", "", .ok, kind: .other, output: "built")),
                Samples.event("q1", Samples.question(.ok)),
            ])
        #expect(Self.rows(log.items) == ["t1", "q1", "u5"])
        let result = try #require(Self.tool(Self.body(log, "t1")))
        #expect(result.status == .ok)
        #expect(result.output == "built")
        #expect(result.name == "Bash")
        #expect(result.kind == .shell)
        #expect(result.title == "make")
        // the question the page saw answered brings no card back
        #expect(log.openQuestion(dismissed: []) == nil)
    }

    @Test("an id on both sides of the page keeps its place and the log's newer word")
    internal func idOnBothSides() throws {
        let log = ChatLog().appending([
            Samples.event("t1", Samples.tool("", "", .ok, kind: .other, output: "built")),
            Samples.event("task", task),
            Samples.event("c1", Samples.tool("Read", "a.rs", .ok, kind: .read), parent: "task"),
            Samples.event("u2", Samples.user("next")),
        ])
        let page = log.prepending([
            Samples.event("u0", Samples.user("start")),
            Samples.event("t1", Samples.tool("Bash", "make")),
            Samples.event("c0", Samples.reply("Reading"), parent: "task"),
            Samples.event("c1", Samples.tool("Read", "a.rs", .running, kind: .read), parent: "task"),
            Samples.event("u2", Samples.user("next")),
        ])
        #expect(Self.rows(page.items) == ["u0", "t1", "task[c0, c1]", "u2"])
        #expect(Self.tool(Self.body(page, "c1"))?.status == .ok)
        let result = try #require(Self.tool(Self.body(page, "t1")))
        #expect(result.status == .ok)
        #expect(result.name == "Bash")
        #expect(result.title == "make")
    }

    @Test("a placeholder whose task an older page brings moves to the task, with its key")
    internal func placeholderMoves() {
        let log = ChatLog().appending([
            Samples.event("c2", Samples.reply("Found it"), parent: "task"),
            Samples.event("a1", Samples.reply("Waiting")),
        ])
        let page = log.prepending([
            Samples.event("u0", Samples.user("start")),
            Samples.event("task", task),
            Samples.event("c1", Samples.reply("Looking"), parent: "task"),
            Samples.event("u1", Samples.user("meanwhile")),
        ])
        #expect(Self.rows(page.items) == ["u0", "task[c1, c2]", "u1", "a1"])
    }

    @Test("a page's subagent events read before their task go under it, at its place")
    internal func pageSubagentBeforeTask() {
        let log = ChatLog()
            .appending([Samples.event("a1", Samples.reply("Done"))])
            .prepending([
                Samples.event("c1", Samples.reply("Looking"), parent: "task"),
                Samples.event("u1", Samples.user("go")),
                Samples.event("task", task),
            ])
        #expect(Self.rows(log.items) == ["u1", "task[c1]", "a1"])
    }

    @Test("a reset leaves the divider, and the new conversation's pages under it")
    internal func reset() {
        let log = ChatLog()
            .appending([Samples.event("u1", Samples.user("old"))])
            .reset()
            .appending([Samples.event("v1", Samples.user("new"))])
            .prepending([Samples.event("v0", Samples.user("before"))])
        #expect(Self.rows(log.items) == ["--", "v0", "v1"])
        #expect(log.items.first?.key == "divider")
    }
}
