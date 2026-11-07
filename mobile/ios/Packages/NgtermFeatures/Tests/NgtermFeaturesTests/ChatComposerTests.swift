import NgtermCore
import Testing

@testable import NgtermFeatures

/// The chat's composer (mobile/android's `ChatComposerTest`): what the field takes, the `/` menu over it, its catalog,
/// and an attached file's path put in at the cursor.
@MainActor
@Suite("the chat's composer")
internal struct ChatComposerTests {
    private let commands = [
        ChatSamples.command("compact", "Clear history but keep a summary"),
        ChatSamples.command("context", "Show how the context window is used"),
        ChatSamples.command("clear", "Start a new conversation"),
    ]

    // a composer over `commands` (or a failure), counting its fetches, ranking by prefix as a stand-in for the core's
    private func composer(failing: Bool = false, fetches: FetchCount = FetchCount()) -> ChatComposer {
        let listed = commands
        return ChatComposer {
            fetches.count += 1
            if failing {
                throw MobileError.Remote(reason: "no agent")
            }
            return listed
        } filter: { query, items in
            items.filter { $0.name.hasPrefix(query) }
        }
    }

    @Test("a / at the start opens the menu, which narrows as it is typed and closes at a space")
    internal func menuOpensAndNarrows() async {
        let composer = composer()
        #expect(composer.menu == nil)
        composer.edit("/")
        await Wait.until { composer.catalog != .fetching }
        guard case .commands(_, let all, 0) = composer.menu else {
            Issue.record("the menu lists the commands: \(String(describing: composer.menu))")
            return
        }
        #expect(all.count == 3)
        composer.edit("/co")
        #expect(composer.menu == .commands(query: "co", commands: Array(commands.prefix(2)), highlighted: 0))
        composer.edit("/compact now")
        #expect(composer.menu == nil, "a space closes it")
        composer.edit("say /compact")
        #expect(composer.menu == nil, "a / that isn't first opens nothing")
    }

    @Test("Up, Down and Return move and pick the highlight; Escape closes the menu until the text changes")
    internal func keys() async {
        let composer = composer()
        composer.edit("/c")
        await Wait.until { composer.catalog != .fetching }
        #expect(composer.moveHighlight(by: 1))
        #expect(composer.moveHighlight(by: 9))
        guard case .commands(_, _, let highlighted) = composer.menu else {
            Issue.record("the menu lists the commands")
            return
        }
        #expect(highlighted == 2, "the highlight stays on the list")
        #expect(composer.pickHighlighted())
        #expect(composer.text == "/clear ")
        composer.edit("/co")
        #expect(composer.dismissMenu())
        #expect(composer.menu == nil)
        #expect(!composer.dismissMenu(), "a closed menu has nothing to close")
        composer.edit("/con")
        #expect(composer.menu != nil, "a change opens it again")
    }

    @Test("the catalog is asked for once per conversation, again after a failure or a new conversation")
    internal func catalogOnce() async {
        let fetches = FetchCount()
        let composer = composer(fetches: fetches)
        composer.edit("/")
        await Wait.until { composer.catalog != .fetching }
        composer.edit("")
        composer.edit("/")
        #expect(fetches.count == 1)
        composer.newConversation()
        await Wait.until { composer.catalog != .fetching }
        #expect(fetches.count == 2, "a new conversation asks again while the menu is open")
        let failing = self.composer(failing: true)
        failing.edit("/")
        await Wait.until { failing.catalog == .failed }
        #expect(failing.menu == .waiting(query: "", isFetching: false), "no list, and no error either")
    }

    @Test("line breaks are taken as the clipboard brings them, and other controls are refused whole")
    internal func controls() {
        let composer = composer()
        #expect(composer.edit("one\r\ntwo\rthree"))
        #expect(composer.text == "one\ntwo\nthree")
        #expect(!composer.edit("bell\u{7}"))
        #expect(composer.text == "one\ntwo\nthree")
    }

    @Test("a picked command goes in as the agent takes it: /name, or a Codex skill's $name")
    internal func insertion() {
        #expect(ChatComposer.insertion(ChatSamples.command("compact")) == "/compact")
        #expect(ChatComposer.insertion(ChatSamples.command("$review")) == "$review")
        #expect(ChatComposer.slashQuery("/co") == "co")
        #expect(ChatComposer.slashQuery("/co x") == nil)
    }

    @Test("an attached file's path goes in at the cursor, spaced from the words, quoted when it has a space")
    internal func insertPath() {
        let composer = composer()
        composer.edit("Look at")
        composer.insertPath(ChatComposer.promptPath("/state/uploads/1/shot.png"))
        #expect(composer.text == "Look at /state/uploads/1/shot.png ")
        composer.edit("")
        composer.insertPath(ChatComposer.promptPath("/state/my shot.png"))
        #expect(composer.text == "\"/state/my shot.png\" ")
    }

    @Test("a sent prompt lets the field go unless it holds something else; a kept draft fills only an empty one")
    internal func sentAndRestored() {
        let composer = composer()
        composer.edit("  Fix the guide ")
        composer.sent("Fix the guide")
        #expect(composer.text.isEmpty)
        composer.restore("draft")
        #expect(composer.text == "draft")
        composer.restore("another")
        #expect(composer.text == "draft")
    }
}
