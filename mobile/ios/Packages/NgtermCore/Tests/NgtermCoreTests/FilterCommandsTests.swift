import NgtermCore
import Testing

/// The `/` menu's filter, as the linked core ranks it (chat.rs's `filter_commands`).
@Suite("the / menu's filter")
internal struct FilterCommandsTests {
    private static let items = [
        command("review", "Review a pull request"),
        command("compact", "Clear conversation history but keep a summary"),
        command("$code-review", "A skill"),
        command("context", "Visualize current context usage"),
    ]

    private static func command(_ name: String, _ description: String) -> AgentCommand {
        AgentCommand(name: name, description: description, kind: .builtin, argumentHint: nil)
    }

    private static func names(_ query: String) -> [String] {
        filterCommands(query: query, items: items).map(\.name)
    }

    @Test("a blank query lists every command, A to Z")
    internal func blankListsAll() {
        #expect(Self.names("") == ["$code-review", "compact", "context", "review"])
        #expect(Self.names("  ") == ["$code-review", "compact", "context", "review"])
    }

    @Test("a query keeps the commands whose name matches, best first")
    internal func queryRanksByName() {
        #expect(Self.names("co") == ["compact", "context", "$code-review"])
        #expect(Self.names("rev") == ["review", "$code-review"])
    }

    @Test("a description's words don't match, and neither does what no name holds")
    internal func noMatch() {
        #expect(Self.names("summary").isEmpty)
        #expect(Self.names("zzz").isEmpty)
    }

    @Test("a command keeps its fields through the filter")
    internal func keepsFields() {
        let skill = AgentCommand(name: "deploy", description: "Ship it", kind: .skill, argumentHint: "[env]")
        #expect(filterCommands(query: "dep", items: [skill]) == [skill])
    }
}
