import NgtermCore
import NgtermCoreFakes
import Testing

@testable import NgtermFeatures

/// A machine running agterm (mobile/android's `ConnectionsTest`, `ChatModeTest` and the screens' agterm cases): the app
/// each machine runs, learned from its listings, what the screens say of it, and the chat view it doesn't have.
@MainActor
@Suite("an agterm machine")
internal struct AgtermPeerTests {
    private let world = SessionWorld()

    private var test: TestGraph {
        world.test
    }

    private var connections: Connections {
        world.test.connections
    }

    // the machine's sessions as `peer` lists them, listed anew
    private func listed(_ peer: PeerKind?) async {
        test.core.sessions["m1"] = [fakeSession(id: "s1", name: "api", peer: peer)]
        await connections.listSessions(machineId: "m1")
        await test.settled()
    }

    @Test("what agterm offers: no chat view, a badge, and the core's note; ngterm offers everything")
    internal func offers() {
        let agterm = MachinePeer(kind: .agterm)
        #expect(agterm.isForeign)
        #expect(!agterm.offersChat)
        #expect(agterm.badge == "agterm")
        #expect(agterm.note?.contains("the agent chat view") == true)
        let ngterm = MachinePeer(kind: .ngterm)
        #expect(!ngterm.isForeign)
        #expect(ngterm.offersChat)
        #expect(ngterm.note == nil)
    }

    @Test("a listing names the machine's app; an empty one keeps it, another app replaces it, a disconnect forgets it")
    internal func learned() async throws {
        defer { world.remove() }
        try await world.attach([], of: ["s1"], peer: .agterm)
        #expect(connections.peers["m1"]?.kind == .agterm)
        test.core.sessions["m1"] = []
        await connections.listSessions(machineId: "m1")
        await test.settled()
        #expect(connections.trees["m1"]?.isEmpty == true)
        #expect(connections.peers["m1"]?.kind == .agterm, "an empty listing says nothing")
        await listed(nil)
        #expect(connections.peers["m1"]?.kind == .agterm, "a listing that doesn't say keeps it")
        await listed(.ngterm)
        #expect(connections.peers["m1"]?.kind == .ngterm)
        connections.disconnect(machineId: "m1")
        #expect(connections.peers["m1"] == nil)
    }

    @Test("a machine edited to reach another host forgets its app until it lists")
    internal func edited() async throws {
        defer { world.remove() }
        try await world.attach([], of: ["s1"], peer: .agterm)
        var moved = TestGraph.machine(keyId: "k1").core
        moved.host = "mini.lan"
        let connecting = connections.connect(moved)
        #expect(connections.peers["m1"] == nil)
        await connecting.value
        await test.settled()
        #expect(connections.peers["m1"]?.kind == .agterm, "its listing named it again")
    }

    @Test("a machine that never says runs ngterm: no badge, the chat view as Settings has it")
    internal func ngtermByDefault() async throws {
        defer { world.remove() }
        try await world.attach()
        #expect(connections.peers["m1"] == nil)
        #expect(test.graph.chats.noChatOn(SessionWorld.key) == nil)
        let choice = SessionModel(graph: test.graph, key: SessionWorld.key).chatView
        #expect(choice.isOn)
        #expect(choice.noChatOn == nil)
        #expect(test.graph.attachedSessions.session(SessionWorld.key)?.peer == nil)
    }

    @Test("an agent's pane on agterm keeps its terminal: no switch, and no chat is opened even when asked for")
    internal func noChat() async throws {
        defer { world.remove() }
        try await world.attach(["s1"], of: ["s1"], peer: .agterm)
        await world.stream(SessionWorld.layout(["P1"], axis: nil, shown: true))
        await world.emit(
            .paneAgent(machineId: "m1", sessionId: "s1", paneId: "P1", agent: "claude", conversationId: "c1")
        )
        await world.show("P1")
        let chats = test.graph.chats
        let pane = SessionWorld.pane("P1")
        #expect(chats.view(of: pane) == nil)
        #expect(chats.noChatOn(SessionWorld.key) == "agterm")
        chats.setChatView(SessionWorld.key, on: true)
        chats.pick(pane, .chat)
        await test.settled()
        #expect(chats.view(of: pane) == nil)
        #expect(chats.chats.isEmpty)
        #expect(!test.core.calls.contains { $0.method == "openChat" })
    }

    @Test("Chat view is off and says agterm has none, in the session's More and in the machine's list")
    internal func chatViewChoices() async throws {
        defer { world.remove() }
        try await world.attach(["s1"], of: ["s1"], peer: .agterm)
        let inMore = SessionModel(graph: test.graph, key: SessionWorld.key).chatView
        #expect(!inMore.isOn)
        #expect(inMore.noChatOn == "agterm")
        let sessions = SessionsModel(graph: test.graph, machineId: "m1", windows: SessionWindows())
        let inList = sessions.chatView(sessionId: "s1")
        #expect(!inList.isOn)
        #expect(inList.noChatOn == "agterm")
    }

    @Test("the screens carry the machine's app: its row, its sessions screen and its attached session")
    internal func screens() async throws {
        defer { world.remove() }
        try await world.attach(["s1"], of: ["s1"], peer: .agterm)
        let item = MachinesModel(graph: test.graph).items.first { $0.id == "m1" }
        #expect(item?.peer?.kind == .agterm)
        let sessions = SessionsModel(graph: test.graph, machineId: "m1", windows: SessionWindows())
        #expect(sessions.state?.peer?.kind == .agterm)
        #expect(test.graph.attachedSessions.session(SessionWorld.key)?.peer?.kind == .agterm)
    }
}
