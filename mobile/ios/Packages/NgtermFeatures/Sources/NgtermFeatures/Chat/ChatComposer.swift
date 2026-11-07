import NgtermCore
import Observation
import SwiftUI

/// A chat's composer (mobile/android's `ChatComposer`): the prompt being written (``text`` and its ``selection``) and
/// the `/` menu over it.
///
/// A `/` at the start of a text with no space or line break in it opens the menu (``menu``) on the agent's commands,
/// narrowed by what follows the `/` through the filter (the core's `filter_commands`, the palettes' ranking) as it is
/// typed; a space, a line break or deleting the `/` closes it, and so does Escape until the text changes. Picking a
/// command (``pick(_:)``) puts `/name ` in the field (a Codex skill's `$name ` as it stands), the cursor at its end.
///
/// The catalog comes once per conversation (asked as the menu first opens, again after a failure when it next opens,
/// or at once after a new conversation while it is open).
@preconcurrency
@MainActor
@Observable
public final class ChatComposer {
    /// What the field holds.
    public private(set) var text = ""
    /// Where the cursor or the selection is in ``text``; `nil` reads as the end.
    public var selection: TextSelection?
    /// The catalog for the conversation on screen.
    public private(set) var catalog = CommandCatalog.unasked
    // the text whose menu Escape closed: it stays closed until the text changes
    private var dismissed: String?
    private var highlight = 0
    @ObservationIgnored private let fetch: @MainActor () async throws -> [AgentCommand]
    @ObservationIgnored private let filter: @MainActor (String, [AgentCommand]) -> [AgentCommand]
    // the catalog asked for last: an answer to an earlier ask (a conversation since) is dropped
    @ObservationIgnored private var asking: Task<Void, Never>?
    @ObservationIgnored private var asks = 0
    // the last ranking, kept while neither the query nor the catalog changes
    @ObservationIgnored private var ranked: (query: String, from: [AgentCommand], to: [AgentCommand])?

    /// What follows the `/` while the text opens the menu (Escape aside), else `nil`.
    public var query: String? {
        Self.slashQuery(text)
    }

    /// The menu as it shows now, or `nil` while it is closed.
    public var menu: SlashMenu? {
        guard let query, text != dismissed else {
            return nil
        }
        switch catalog {
        case .ready(let commands):
            guard !commands.isEmpty else {
                return .waiting(query: query, isFetching: false)
            }
            let rows = rank(query, commands)
            return .commands(query: query, commands: rows, highlighted: min(highlight, max(rows.count - 1, 0)))

        case .fetching:
            return .waiting(query: query, isFetching: true)

        case .unasked, .failed:
            return .waiting(query: query, isFetching: false)
        }
    }

    /// The composer whose menu asks `fetch` for the agent's commands and ranks them with `filter`.
    @preconcurrency
    public init(
        fetch: @escaping @MainActor () async throws -> [AgentCommand],
        filter: @escaping @MainActor (String, [AgentCommand]) -> [AgentCommand] = { query, items in
            filterCommands(query: query, items: items)
        }
    ) {
        self.fetch = fetch
        self.filter = filter
    }

    /// What follows a leading `/` in `text` while the text is that and one word, else `nil`.
    public static func slashQuery(_ text: String) -> String? {
        guard text.hasPrefix("/"), !text.contains(where: \.isWhitespace) else {
            return nil
        }
        return String(text.dropFirst())
    }

    /// What picking `command` puts in the field, before its space: `/name`, or a Codex skill's `$name` as it stands.
    public static func insertion(_ command: AgentCommand) -> String {
        command.name.hasPrefix("$") || command.name.hasPrefix("/") ? command.name : "/\(command.name)"
    }

    /// `path` as it goes into a prompt: as is, or in double quotes when it holds a space, so the agent reads one path.
    public static func promptPath(_ path: String) -> String {
        path.contains(where: \.isWhitespace) ? "\"\(path)\"" : path
    }

    /// The field takes `typed`, and says whether it took it.
    ///
    /// Line breaks as the clipboard brings them (`\r\n`, `\r`) become `\n`, and an edit that would add another control
    /// character is refused whole. The menu opens or narrows with it.
    @discardableResult
    public func edit(_ typed: String) -> Bool {
        let next = typed.replacing("\r\n", with: "\n").replacing("\r", with: "\n")
        guard !AskText.hasForbiddenControls(next, multiline: true) else {
            return false
        }
        let opened = query == nil && Self.slashQuery(next) != nil
        if next != text {
            highlight = 0
            dismissed = nil
        }
        text = next
        if query != nil {
            ask(retry: opened)
        }
        return true
    }

    /// Puts `command` in the field in place of the `/` and the query, then a space, the cursor after it.
    public func pick(_ command: AgentCommand) {
        set(Self.insertion(command) + " ")
        highlight = 0
    }

    /// Picks the highlighted command; `false` when the menu lists none (closed, waiting, or nothing matches).
    @discardableResult
    public func pickHighlighted() -> Bool {
        guard case .commands(_, let commands, let highlighted) = menu, commands.indices.contains(highlighted) else {
            return false
        }
        pick(commands[highlighted])
        return true
    }

    /// Moves the highlight `by` rows, staying on the list; `false` when the menu lists none.
    @discardableResult
    public func moveHighlight(by step: Int) -> Bool {
        guard case .commands(_, let commands, let highlighted) = menu, !commands.isEmpty else {
            return false
        }
        highlight = min(max(highlighted + step, 0), commands.count - 1)
        return true
    }

    /// Closes the menu (Escape) until the text changes; `false` when it was closed.
    @discardableResult
    public func dismissMenu() -> Bool {
        guard menu != nil else {
            return false
        }
        dismissed = text
        return true
    }

    /// Starts a command: a `/` in the empty field, which opens the menu.
    public func startCommand() {
        set("/")
        ask(retry: true)
    }

    /// Puts an attached file's `path` at the cursor (in place of a selection), a space between it and the words on
    /// either side, the cursor after it.
    public func insertPath(_ path: String) {
        let (start, end) = selectedOffsets()
        let scalars = Array(text.utf16)
        let before = String(utf16CodeUnits: Array(scalars[..<start]), count: start)
        var after = String(utf16CodeUnits: Array(scalars[end...]), count: scalars.count - end)
        while after.hasPrefix(" ") {
            after.removeFirst()
        }
        let lead = before.isEmpty || before.last?.isWhitespace == true ? "" : " "
        let inserted = lead + path + " "
        edit(before + inserted + after)
        let offset = min(before.utf16.count + inserted.utf16.count, text.utf16.count)
        selection = TextSelection(insertionPoint: text.utf16.index(text.startIndex, offsetBy: offset))
    }

    /// `sent` went to the agent: the field lets it go, unless it holds something else by now.
    internal func sent(_ sent: String) {
        if text.trimmingCharacters(in: .whitespacesAndNewlines) == sent {
            set("")
        }
    }

    /// Puts a `kept` text from before back in the field, unless the field holds something already.
    internal func restore(_ kept: String) {
        if text.isEmpty, edit(kept) {
            selection = TextSelection(insertionPoint: text.endIndex)
        }
    }

    /// A new conversation: its catalog is asked for afresh, at once while the menu is open.
    internal func newConversation() {
        asking?.cancel()
        asks += 1
        catalog = .unasked
        ranked = nil
        if query != nil {
            ask(retry: false)
        }
    }

    // the field's text set from here, the cursor at its end
    private func set(_ next: String) {
        text = next
        dismissed = nil
        selection = TextSelection(insertionPoint: text.endIndex)
    }

    // the selection's ends as UTF-16 offsets into the text, held within it; the end without one
    private func selectedOffsets() -> (Int, Int) {
        let count = text.utf16.count
        // a selection made in an earlier text (the field emptied since) may lie past this one's end: the end then
        guard case .selection(let range) = selection?.indices, range.upperBound <= text.endIndex else {
            return (count, count)
        }
        let start = min(max(range.lowerBound.utf16Offset(in: text), 0), count)
        let end = min(max(range.upperBound.utf16Offset(in: text), start), count)
        return (start, end)
    }

    // asks for the catalog when there is none for this conversation; `retry` asks again after a failure too
    private func ask(retry: Bool) {
        let isDue = catalog == .unasked || (retry && catalog == .failed)
        guard isDue else {
            return
        }
        asks += 1
        let ask = asks
        catalog = .fetching
        asking = Task { [weak self, fetch] in
            let got: CommandCatalog
            do {
                got = .ready(try await fetch())
            } catch {
                // an older origin, a link that went, or the core stopped: no list, and no error either
                got = .failed
            }
            guard let self, ask == asks, !Task.isCancelled else {
                return
            }
            catalog = got
        }
    }

    private func rank(_ query: String, _ commands: [AgentCommand]) -> [AgentCommand] {
        if let ranked, ranked.query == query, ranked.from == commands {
            return ranked.to
        }
        let rows = filter(query, commands)
        ranked = (query, commands, rows)
        return rows
    }

    isolated deinit {
        asking?.cancel()
    }
}
