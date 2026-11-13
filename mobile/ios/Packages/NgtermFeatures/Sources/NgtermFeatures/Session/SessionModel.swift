import NgtermCore
import NgtermTerminal
import Observation

/// One session view's session (mobile/android's `SessionViewModel`, for the session it shows): what it shows, its
/// panes and the one selected, Take control, Detach and End session.
///
/// The selection lives in the app's ``SessionPanes``, so a new view of the session (another size class, a trip
/// elsewhere and back) finds the same pane. ``notice`` carries an End that failed, for a banner.
@preconcurrency
@MainActor
@Observable
public final class SessionModel {
    /// The session.
    public let key: SessionKey
    /// Whether its End is under way.
    public private(set) var isEnding = false
    /// What the view should say in a banner, until it is dismissed.
    public private(set) var notice: String?
    /// The keyboard every pane of the session types through.
    @ObservationIgnored public let keyboard: TerminalKeyboard
    @ObservationIgnored private let graph: AppGraph

    /// What the view shows: the session live while attached (unless a pane says it is gone), gone once its machine's
    /// listing has dropped it, and `nil` once it was let go some other way (a detach, a disconnect).
    public var state: OpenSession? {
        let session = graph.attachedSessions.session(key)
        if let session, !session.live.panes.values.contains(where: { $0.end == .gone }) {
            return .live(session, focused: focused(in: session))
        }
        let tree = graph.connections.trees[key.machineId]
        guard session != nil || tree?.contains(where: { $0.id == key.sessionId }) == false else {
            return nil
        }
        let last = graph.attachedSessions.lastSeen(key)
        let machine = graph.machines.machine(id: key.machineId)?.title
        return .gone(name: last?.name ?? key.sessionId, machine: last?.machine ?? machine ?? key.machineId)
    }

    /// The selected pane: the one every key goes to; `nil` before the layout arrives, or once the session is gone.
    public var focusedPane: String? {
        guard case .live(_, let focused) = state else {
            return nil
        }
        return focused
    }

    /// Whether the session shows its selected pane alone where the window could show the origin's split.
    public var isAlone: Bool {
        graph.panes.alone.contains(key)
    }

    /// Whether the session's panes running an agent show as a chat, and the way to set it for this session alone.
    public var chatView: ChatViewChoice {
        let chats = graph.chats
        let session = key
        return ChatViewChoice(isOn: chats.isChatView(session), noChatOn: chats.noChatOn(session)) { isOn in
            chats.setChatView(session, on: isOn)
        }
    }

    /// The session `key` over `graph`, its panes typing through `keyboard` (one of its own by default).
    public init(graph: AppGraph, key: SessionKey, keyboard: TerminalKeyboard? = nil) {
        self.graph = graph
        self.key = key
        self.keyboard = keyboard ?? TerminalKeyboard()
    }

    /// The panes of the session's layout, in its order, as the view places them, from what `session` says.
    public func spots(of session: AttachedSession) -> [PaneSpot] {
        let isTaking = graph.panes.isTakingLead(key)
        return (session.live.layout?.panes ?? []).map { paneId in
            let ref = PaneRef(machineId: key.machineId, sessionId: key.sessionId, paneId: paneId)
            let isChat = graph.chats.view(of: ref) == .chat
            return PaneSpot(
                ref: ref,
                shown: graph.panes.shown[ref],
                live: session.live.panes[paneId],
                failure: graph.panes.failures[ref],
                isChatView: isChat,
                chat: isChat ? graph.chats.chats[ref] : nil,
                isTakingLead: isTaking
            )
        }
    }

    /// Selects the pane `paneId`; whether the selection moved.
    @discardableResult
    public func select(_ paneId: String) -> Bool {
        guard case .live = state, focusedPane != paneId else {
            return false
        }
        graph.panes.select(paneId, of: key)
        return true
    }

    /// A tap on the `paneId` pane's tab; whether the panes' views change (the selection moved, the split came or went).
    ///
    /// It selects that pane, and where the window can show the origin's split (`isSplittable`) it toggles the selected
    /// pane alone (``PaneAlone/afterTap(isSplittable:isAlone:tapped:focused:)``).
    @discardableResult
    public func tapTab(_ paneId: String, isSplittable: Bool) -> Bool {
        let wasAlone = isAlone
        let next = PaneAlone.afterTap(
            isSplittable: isSplittable,
            isAlone: wasAlone,
            tapped: paneId,
            focused: focusedPane
        )
        if next != wasAlone {
            graph.panes.setAlone(next, of: key)
        }
        let moved = select(paneId)
        return moved || next != wasAlone
    }

    /// The pane after (or before) the selected one, wrapping around; `nil` with fewer than two.
    public func neighbor(forward: Bool) -> String? {
        guard case .live(let session, let focused) = state else {
            return nil
        }
        return (session.live.layout?.panes ?? []).cyclicNeighbor(of: focused, forward: forward)
    }

    /// The attached session after (or before) this one in the lists' order, wrapping around; `nil` with no other.
    public func neighborSession(forward: Bool) -> SessionKey? {
        graph.attachedSessions.neighbor(of: key, forward: forward)
    }

    /// Shows the pane `spot` at `size`, as its view first has room for it.
    public func show(_ spot: PaneSpot, size: PaneSize) {
        graph.panes.show(spot.ref, size: size)
    }

    /// Tries the pane `spot` again at `size`, after it could not be shown.
    public func retry(_ spot: PaneSpot, size: PaneSize) {
        graph.panes.retry(spot.ref, size: size)
    }

    /// Take control of the pane `spot`.
    public func takeLead(_ spot: PaneSpot) {
        graph.panes.takeLead(spot.ref)
    }

    /// How the pane `paneId` shows: `nil` without an agent, or with the session's chat view off (no switch then).
    public func view(of paneId: String) -> PaneView? {
        graph.chats.view(of: PaneRef(machineId: key.machineId, sessionId: key.sessionId, paneId: paneId))
    }

    /// Shows the pane `paneId` as `view`: its terminal held until its next conversation, or its chat again.
    public func pick(_ paneId: String, _ view: PaneView) {
        graph.chats.pick(PaneRef(machineId: key.machineId, sessionId: key.sessionId, paneId: paneId), view)
    }

    /// Detaches the session: it goes on running there, and the view closes.
    public func detach() {
        graph.connections.detach(machineId: key.machineId, sessionId: key.sessionId)
    }

    /// Ends the session on its machine, with no undo (the view confirms first); whether it ended.
    ///
    /// A failure is the ``notice``.
    @discardableResult
    public func end() async -> Bool {
        guard !isEnding else {
            return false
        }
        isEnding = true
        defer { isEnding = false }
        do {
            try await graph.connections.endSession(machineId: key.machineId, sessionId: key.sessionId)
            return true
        } catch let error as MobileError {
            notice = error.reason
        } catch {
            // the caller's task was cancelled: the End goes on, and the listing shows its outcome
        }
        return false
    }

    /// The ``notice`` was dismissed.
    public func dismissNotice() {
        notice = nil
    }

    // the selected pane: the one picked last while the layout still has it, else the layout's primary
    private func focused(in session: AttachedSession) -> String? {
        guard let layout = session.live.layout else {
            return nil
        }
        if let picked = graph.panes.focus[key], layout.panes.contains(picked) {
            return picked
        }
        return layout.primary
    }

    deinit {
        // nothing to release
    }
}
