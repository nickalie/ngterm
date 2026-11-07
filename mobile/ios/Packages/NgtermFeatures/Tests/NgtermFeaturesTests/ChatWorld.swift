import NgtermCore
import NgtermCoreFakes

@testable import NgtermFeatures

/// A ``SessionWorld`` whose pane `P1` runs Claude Code in conversation `c1`, shown, with its chat open.
@MainActor
internal final class ChatWorld {
    /// The pane with the agent.
    internal static let pane = SessionWorld.pane("P1")

    /// The session world under it.
    internal let world = SessionWorld()

    /// The app's graph.
    internal var graph: AppGraph {
        world.graph
    }

    /// The fake core.
    internal var core: FakeMobileCore {
        world.core
    }

    /// The pane's chat, once open.
    internal var chat: ChatModel? {
        graph.chats.chats[Self.pane]
    }

    /// The pane's handle, once shown.
    internal var fakePane: FakePane? {
        graph.panes.shown[Self.pane]?.client as? FakePane
    }

    /// The session attached and streaming its split, `P1` announced running Claude Code in `c1` and shown, and its
    /// chat open.
    internal func open() async throws {
        try await made()
        await Wait.until { self.core.openChats.contains(Self.pane) }
    }

    /// As ``open()``, its chat made but not necessarily opened at the core.
    internal func made() async throws {
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        await announce("c1")
        await world.show("P1")
        await Wait.until { self.chat != nil }
    }

    /// The origin announces Claude Code in `conversation` (`nil`: no agent) in `P1`.
    internal func announce(_ conversation: String?) async {
        await world.emit(
            .paneAgent(
                machineId: "m1",
                sessionId: "s1",
                paneId: "P1",
                agent: conversation == nil ? nil : "claude",
                conversationId: conversation
            )
        )
    }

    /// The chat's events, as its channel streams them.
    internal func chat(_ events: ChatEvent...) async {
        await world.emit(.chat(pane: Self.pane, events: events))
    }

    /// Deletes the graph's directory.
    internal func remove() {
        world.remove()
    }

    deinit {
        // the directory goes with remove()
    }
}
