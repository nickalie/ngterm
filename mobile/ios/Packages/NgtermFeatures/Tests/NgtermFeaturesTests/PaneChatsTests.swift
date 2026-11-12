import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// Which panes show as a chat, and their chats (mobile/android's `ChatModeTest`): the origin's announcement, Settings'
/// Agent chat view and the session's own, a terminal picked, a chat that ended, and the chats kept open for the shown
/// panes only.
@MainActor
@Suite("the panes' chats")
internal struct PaneChatsTests {
    private let world = ChatWorld()
    private let key = SessionWorld.key

    private var chats: PaneChats {
        world.graph.chats
    }

    @Test("a pane its origin announces an agent in shows as a chat, its chat open once shown; another has no switch")
    internal func announced() async throws {
        defer { world.remove() }
        try await world.open()
        #expect(chats.view(of: ChatWorld.pane) == .chat)
        #expect(chats.view(of: SessionWorld.pane("P2")) == nil)
        #expect(world.chat != nil)
        // the agent leaves: no chat, no switch
        await world.announce(nil)
        await Wait.until { world.chat == nil }
        #expect(chats.view(of: ChatWorld.pane) == nil)
        #expect(!world.core.openChats.contains(ChatWorld.pane))
    }

    @Test("Settings' Agent chat view off shows the terminal; a session's own choice outranks it, and is kept")
    internal func chatViews() async throws {
        defer { world.remove() }
        try await world.open()
        try world.graph.settings.update { $0.agentChat = false }
        await Wait.until { world.chat == nil }
        #expect(chats.view(of: ChatWorld.pane) == nil)
        chats.setChatView(key, on: true)
        await Wait.until { world.chat != nil }
        #expect(chats.isChatView(key))
        let stored = { GraphStores(directory: world.world.test.directory).sessionChatViews }
        #expect(stored().choice(machine: "m1", session: "s1")?.chat == true)
        // set back to Settings' own, it follows Settings again
        chats.setChatView(key, on: false)
        #expect(stored().views.isEmpty)
    }

    @Test("a terminal picked holds until the pane's next conversation; picking the chat lifts it")
    internal func picked() async throws {
        defer { world.remove() }
        try await world.open()
        chats.pick(ChatWorld.pane, .terminal)
        await Wait.until { world.chat == nil }
        #expect(chats.view(of: ChatWorld.pane) == .terminal)
        chats.pick(ChatWorld.pane, .chat)
        await Wait.until { world.chat != nil }
        chats.pick(ChatWorld.pane, .terminal)
        await world.announce("c2")
        await Wait.until { world.chat != nil }
        #expect(chats.view(of: ChatWorld.pane) == .chat, "the next conversation starts in chat view")
    }

    @Test("a chat that ended shows the terminal until the next conversation")
    internal func ended() async throws {
        defer { world.remove() }
        try await world.open()
        await world.chat(ChatSamples.event("conv", ChatSamples.conversation))
        await world.world.emit(.chatState(pane: ChatWorld.pane, state: .ended))
        await Wait.until { world.chat == nil }
        #expect(chats.view(of: ChatWorld.pane) == .terminal)
        await world.announce("c2")
        await Wait.until { world.chat != nil }
    }

    @Test("a chat that ends as its pane moves to the next conversation opens again for that one")
    internal func endedThenNext() async throws {
        defer { world.remove() }
        try await world.open()
        await world.chat(ChatSamples.event("conv", ChatSamples.conversation))
        // /clear: the old chat ends in the moment the next conversation is announced, both before the chats look
        world.core.emit(.chatState(pane: ChatWorld.pane, state: .ended))
        await world.announce("c2")
        await Wait.until { world.chat.map { $0.snapshot.phase != .ended } ?? false }
        #expect(chats.view(of: ChatWorld.pane) == .chat)
        #expect(world.core.calls.count { $0.method == "openChat" } == 2, "the next conversation's chat opened")
    }

    @Test("a chat's events reach it, and a notification's tap brings it to its end")
    internal func events() async throws {
        defer { world.remove() }
        try await world.open()
        await world.chat(
            ChatSamples.event("conv", ChatSamples.conversation),
            ChatSamples.event("u1", ChatSamples.user("Fix the guide")),
            ChatSamples.event("live", .live)
        )
        let chat = try #require(world.chat)
        #expect(chat.snapshot.phase == .live)
        #expect(chat.snapshot.items.map(\.key) == ["e:u1"])
        #expect(chat.snapshot.conversation?.id == "c1")
        chats.reveal(ChatWorld.pane)
        #expect(chat.snapshot.toEnd)
        chat.endShown()
        #expect(!chat.snapshot.toEnd)
    }

    @Test("a chat asked for while the link is down opens once it is back")
    internal func linkDown() async throws {
        defer { world.remove() }
        world.core.failNext("openChat", with: .NotConnected(machineId: "m1"))
        try await world.made()
        let chat = try #require(world.chat)
        await Wait.until { chat.snapshot.phase == .reconnecting }
        await world.world.emit(.machineState(machineId: "m1", state: .connected))
        await Wait.until { world.core.openChats.contains(ChatWorld.pane) }
        #expect(chat.snapshot.phase == .loading)
    }

    @Test("a draft left in a chat that closes comes back to the pane's next chat of its conversation")
    internal func draftKept() async throws {
        defer { world.remove() }
        try await world.open()
        await world.chat(ChatSamples.event("conv", ChatSamples.conversation))
        world.chat?.composer.edit("half a prompt")
        chats.pick(ChatWorld.pane, .terminal)
        await Wait.until { world.chat == nil }
        chats.pick(ChatWorld.pane, .chat)
        await Wait.until { world.chat != nil }
        await world.chat(ChatSamples.event("conv", ChatSamples.conversation))
        await Wait.until { world.chat?.composer.text == "half a prompt" }
    }

    @Test("a prompt on its way when its chat closes comes back with the draft, ahead of what was typed since")
    internal func draftKeepsPromptOnItsWay() async throws {
        defer { world.remove() }
        try await world.open()
        await world.chat(ChatSamples.event("conv", ChatSamples.conversation), ChatSamples.event("live", .live))
        let pane = try #require(world.fakePane)
        let gate = FakeGate<Void>()
        pane.gate = gate
        pane.outcome = .dialogOpen
        world.chat?.composer.edit("Fix the guide")
        world.chat?.send()
        world.chat?.composer.edit("and the tests")
        chats.pick(ChatWorld.pane, .terminal)
        await Wait.until { world.chat == nil }
        gate.open()
        chats.pick(ChatWorld.pane, .chat)
        await Wait.until { world.chat != nil }
        await world.chat(ChatSamples.event("conv", ChatSamples.conversation))
        await Wait.until { world.chat?.composer.text == "Fix the guide\nand the tests" }
    }
}
