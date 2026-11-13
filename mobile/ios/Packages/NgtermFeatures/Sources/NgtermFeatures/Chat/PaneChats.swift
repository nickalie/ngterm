import NgtermCore
import NgtermDesign
import NgtermPlatform
import Observation

/// Which panes show as a chat, and their chats (mobile/android's `PaneChats`).
///
/// A pane whose origin announces an agent (``LiveSession/agents``) shows as a chat while its session's chat view is on
/// (``isChatView(_:)``: Settings' Agent chat view, unless the session's More menu set its own through
/// ``setChatView(_:on:)``), unless its terminal was picked: by hand (``pick(_:_:)``), or because its chat ended or the
/// origin couldn't open it (a link that is down only defers the open until it is back). A terminal so picked holds
/// until the pane's next conversation (or the agent leaves); picking the chat again lifts it. With its session's chat
/// view off every pane shows its terminal, as on a machine whose app has no chat view (agterm:
/// ``Connections/peers``), where no chat is ever opened.
///
/// A shown pane (``SessionPanes/shown``) in chat view has its chat open in ``chats``, so the core's chat channels stay
/// as few as the panes shown; a pane leaving chat view (or being hidden) closes its chat, the pane's own attach staying
/// open behind it all along. A draft left in a chat that closes comes back to the pane's next chat of the same
/// conversation.
@preconcurrency
@MainActor
@Observable
public final class PaneChats {
    // a pane's terminal, picked for a conversation (nil: the one still to be named)
    private struct Hold: Equatable {
        let conversation: String?
    }

    // a draft left in a chat that closed, with the conversation it was written for
    private struct Draft {
        let conversation: String
        let text: String
    }

    // what the chats follow: each agent announced, whether its session's chat view is on, and the shown panes
    private struct Facts: Equatable, Sendable {
        let agents: [PaneRef: PaneAgent]
        let viewsOn: Set<SessionKey>
        let shown: Set<PaneRef>
    }

    /// The open chats, by pane.
    public private(set) var chats: [PaneRef: ChatModel] = [:]
    private var holds: [PaneRef: Hold] = [:]
    // the sessions' choices the store could not keep, and those set back to Settings' it could not drop, over what it
    // reads until the process ends or a later write of the same session lands
    private var unsaved: [SessionKey: Bool] = [:]
    private var unsavedDefault: Set<SessionKey> = []
    @ObservationIgnored private let connections: Connections
    @ObservationIgnored private let live: LiveSessions
    @ObservationIgnored private let panes: SessionPanes
    @ObservationIgnored private let settings: SettingsStore
    @ObservationIgnored private let store: SessionChatViewsStore
    @ObservationIgnored private let filter: @MainActor (String, [AgentCommand]) -> [AgentCommand]
    // how many times each machine's link has come up, by id: a chat asked for while it was down opens on the next
    @ObservationIgnored private var connects: [String: Int] = [:]
    @ObservationIgnored private var drafts: [PaneRef: Draft] = [:]
    @ObservationIgnored private var following: Task<Void, Never>?

    /// The chats over `connections`, of the agents `live` announces in the panes `panes` shows, each session's chat
    /// view Settings' (`settings`) unless `store` keeps its own; a `/` menu ranks with `filter`.
    @preconcurrency
    public init(
        connections: Connections,
        live: LiveSessions,
        panes: SessionPanes,
        settings: SettingsStore,
        store: SessionChatViewsStore,
        filter: @escaping @MainActor (String, [AgentCommand]) -> [AgentCommand] = { query, items in
            filterCommands(query: query, items: items)
        }
    ) {
        self.connections = connections
        self.live = live
        self.panes = panes
        self.settings = settings
        self.store = store
        self.filter = filter
        connections.listen { [weak self] event in
            self?.receive(event)
        }
        following = Task { [weak self] in
            let facts = Observations { @MainActor [weak self] in self?.facts() }
            for await now in facts {
                guard let self, let now else {
                    return
                }
                follow(now)
            }
        }
    }

    // whether the session's agent status says the agent in `paneId` works on a turn: the status is the session's,
    // naming the pane it came from (none: an origin that doesn't say, taken as the pane's)
    private static func isWorking(_ session: LiveSession?, paneId: String) -> Bool {
        guard let status = session?.status, AgentState.fromWire(status.status) == .active else {
            return false
        }
        return status.pane == nil || status.pane == paneId
    }

    /// Whether the panes of `key` running an agent show as a chat: the session's own choice, else Settings'.
    public func isChatView(_ key: SessionKey) -> Bool {
        if unsavedDefault.contains(key) {
            return settings.settings.agentChat
        }
        if let kept = unsaved[key] {
            return kept
        }
        return store.choice(machine: key.machineId, session: key.sessionId)?.chat ?? settings.settings.agentChat
    }

    /// The name of the app without a chat view that the machine of `key` runs, or `nil` when it has one.
    public func noChatOn(_ key: SessionKey) -> String? {
        guard let peer = connections.peers[key.machineId], !peer.offersChat else {
            return nil
        }
        return peer.badge
    }

    /// How `pane` shows: `nil` for a pane with no agent, or whose session's chat view is off or whose machine's app has
    /// none (no switch then).
    public func view(of pane: PaneRef) -> PaneView? {
        let key = SessionKey(pane)
        guard isChatView(key), noChatOn(key) == nil, let agent = live.sessions[key]?.agents[pane.paneId] else {
            return nil
        }
        if let hold = holds[pane], hold.conversation == nil || hold.conversation == agent.conversation {
            return .terminal
        }
        return .chat
    }

    /// Shows `pane` as `view`: its terminal held until its next conversation, or its chat again.
    public func pick(_ pane: PaneRef, _ view: PaneView) {
        switch view {
        case .terminal:
            hold(pane)

        case .chat:
            holds[pane] = nil
        }
    }

    /// Shows the panes of `key` running an agent as a chat (`isOn`) or as their terminal, whatever Settings' default.
    ///
    /// A choice equal to the default follows it from then on. A choice the store can't keep holds until the app ends.
    public func setChatView(_ key: SessionKey, on isOn: Bool) {
        let fallback = settings.settings.agentChat
        unsaved[key] = nil
        unsavedDefault.remove(key)
        do {
            try store.set(machine: key.machineId, session: key.sessionId, chat: isOn, default: fallback)
        } catch {
            if isOn == fallback {
                unsavedDefault.insert(key)
            } else {
                unsaved[key] = isOn
            }
        }
    }

    /// Brings the open chat of `pane` to its newest row, the card it waits on pinned under it.
    ///
    /// A notification's tap opened the pane. A pane with no chat open has nothing to move: it shows its terminal, or
    /// its chat opens at its end.
    public func reveal(_ pane: PaneRef) {
        chats[pane]?.showEnd()
    }

    private func facts() -> Facts {
        var agents: [PaneRef: PaneAgent] = [:]
        var viewsOn: Set<SessionKey> = []
        for (key, session) in live.sessions {
            if isChatView(key), noChatOn(key) == nil {
                viewsOn.insert(key)
            }
            for (paneId, agent) in session.agents {
                agents[PaneRef(machineId: key.machineId, sessionId: key.sessionId, paneId: paneId)] = agent
            }
        }
        // the holds are read too, so a pick or a lifted hold opens or closes a chat at once
        _ = holds
        return Facts(agents: agents, viewsOn: viewsOn, shown: Set(panes.shown.keys))
    }

    private func follow(_ facts: Facts) {
        keepHolds(facts.agents)
        sync(facts)
    }

    // a pane whose agent left holds nothing: its next agent starts in chat view
    private func hold(_ pane: PaneRef) {
        guard let agent = live.sessions[SessionKey(pane)]?.agents[pane.paneId] else {
            return
        }
        holds[pane] = Hold(conversation: agent.conversation)
    }

    // a hold lasts while the pane's agent stays in the conversation it was made in; one made before the conversation
    // was named takes the first name it gets
    private func keepHolds(_ agents: [PaneRef: PaneAgent]) {
        var kept: [PaneRef: Hold] = [:]
        for (pane, hold) in holds {
            guard let agent = agents[pane] else {
                continue
            }
            switch hold.conversation {
            case nil:
                kept[pane] = Hold(conversation: agent.conversation)

            case agent.conversation:
                kept[pane] = hold

            default:
                continue
            }
        }
        if kept != holds {
            holds = kept
        }
    }

    // the chats open for exactly the shown panes in chat view
    private func sync(_ facts: Facts) {
        let wanted = Set(
            facts.agents.keys.filter { pane in
                facts.shown.contains(pane) && facts.viewsOn.contains(SessionKey(pane)) && view(of: pane) == .chat
            }
        )
        for (pane, chat) in chats where !wanted.contains(pane) {
            close(chat)
        }
        var open = chats.filter { wanted.contains($0.key) }
        // a chat that ended and is still wanted is one the pane moved on from (its next conversation lifted the hold
        // the end made, before this pass ran): it goes, and the pane's next chat opens in its place
        for (pane, chat) in open where chat.snapshot.phase == .ended {
            close(chat)
            open[pane] = nil
        }
        var made: [ChatModel] = []
        let order = wanted.sorted { ($0.machineId, $0.sessionId, $0.paneId) < ($1.machineId, $1.sessionId, $1.paneId) }
        for pane in order where open[pane] == nil {
            let chat = chat(for: pane, opened: facts.agents[pane]?.conversation)
            open[pane] = chat
            made.append(chat)
        }
        if open.keys != chats.keys || !made.isEmpty {
            chats = open
        }
        for chat in made {
            chat.open()
            if let draft = drafts.removeValue(forKey: chat.pane) {
                chat.restoreDraft(conversation: draft.conversation, text: draft.text)
            }
        }
    }

    private func close(_ chat: ChatModel) {
        let text = chat.composer.draft
        if let conversation = chat.snapshot.conversation?.id, !AskText.isBlank(text) {
            drafts[chat.pane] = Draft(conversation: conversation, text: text)
        }
        chat.close()
    }

    private func chat(for pane: PaneRef, opened: String?) -> ChatModel {
        let key = SessionKey(pane)
        let machineId = pane.machineId
        let sources = ChatModel.Sources(
            core: connections.core,
            handle: { [weak self] in self?.panes.shown[pane]?.client },
            isWorking: { [weak self] in Self.isWorking(self?.live.sessions[key], paneId: pane.paneId) },
            connects: { [weak self] in self?.connects[machineId] ?? 0 },
            onEnded: { [weak self] chat in
                self?.ended(chat, opened: opened)
            }
        )
        return ChatModel(pane: pane, sources: sources, filter: filter)
    }

    // a chat that ended, or never opened, leaves its terminal on screen until the next conversation: the one it
    // followed (its events named, or the pane announced as it opened). The pane may have moved on meanwhile (/clear
    // ends the old conversation's chat in the moment its record is cleared, and the next is announced right after):
    // then the chat goes and, the pane still in chat view, opens again for the conversation announced now
    private func ended(_ chat: ChatModel, opened: String?) {
        let pane = chat.pane
        guard chats[pane] === chat, let agent = live.sessions[SessionKey(pane)]?.agents[pane.paneId] else {
            return
        }
        let followed = chat.snapshot.conversation?.id ?? opened
        if followed == nil || agent.conversation == followed {
            holds[pane] = Hold(conversation: agent.conversation)
        } else {
            close(chat)
            chats[pane] = nil
            sync(facts())
        }
    }

    private func receive(_ event: MobileEvent) {
        switch event {
        case .chat(let pane, let events):
            chats[pane]?.take(events)

        case .chatState(let pane, let state):
            chats[pane]?.take(state)

        case .machineState(let machineId, .connected):
            connects[machineId, default: 0] += 1
            for (pane, chat) in chats where pane.machineId == machineId {
                chat.linkCameBack()
            }

        default:
            break
        }
    }

    isolated deinit {
        following?.cancel()
    }
}
