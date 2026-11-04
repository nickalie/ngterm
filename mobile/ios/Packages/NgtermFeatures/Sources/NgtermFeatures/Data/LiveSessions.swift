import NgtermCore
import NgtermPlatform
import Observation

/// What the core tells the app about the attached sessions and their shown panes (mobile/android's `LiveSessions`).
///
/// It keeps, per session in ``sessions``, the stream's state, status, context, layout and HUD, and each pane's role,
/// cover, end and title. A shown pane's frames go to the ``PaneFrames`` ``route(_:to:)``d to it. A detached session's
/// state is let go, and the attached set is written to the ``AttachedStore`` on every change, for a background refresh
/// that starts with nothing attached. A stream that fails before the session was ever shown has the machine listed
/// again, so a session ended elsewhere since its listing leaves.
@preconcurrency
@MainActor
@Observable
public final class LiveSessions {
    /// Each attached session's live state, by key; one with no event yet is missing.
    public private(set) var sessions: [SessionKey: LiveSession] = [:]
    @ObservationIgnored private let connections: Connections
    @ObservationIgnored private let store: AttachedStore
    @ObservationIgnored private var frames: [PaneRef: any PaneFrames] = [:]
    // the attached set as last seen: only what was in it and no longer is goes (an event may land before its attach is
    // recorded)
    @ObservationIgnored private var attached: Set<SessionKey> = []

    /// The live sessions over `connections`, whose events they take from now on, writing the attached set to `store`.
    public init(connections: Connections, store: AttachedStore) {
        self.connections = connections
        self.store = store
        connections.listen { [weak self] event in
            self?.receive(event)
        }
        connections.watchAttached { [weak self] attached in
            self?.attachedChanged(attached)
        }
    }

    /// Sends the frames of `pane` to `frames` from now on; `nil` stops them.
    public func route(_ pane: PaneRef, to frames: (any PaneFrames)?) {
        self.frames[pane] = frames
    }

    private func attachedChanged(_ now: [String: Set<String>]) {
        // a set the store can't write leaves the last one written: a refresh then looks at the sessions it names
        try? store.save(now)
        let keys = Set(now.flatMap { machineId, ids in ids.map { SessionKey(machineId: machineId, sessionId: $0) } })
        let gone = attached.subtracting(keys)
        attached = keys
        for key in gone {
            sessions[key] = nil
        }
    }

    private func receive(_ event: MobileEvent) {
        switch event {
        case .sessionConnection(let machineId, let sessionId, let state):
            connectionChanged(SessionKey(machineId: machineId, sessionId: sessionId), state)

        case .sessionStatus(let machineId, let sessionId, let status, _):
            change(SessionKey(machineId: machineId, sessionId: sessionId)) { session in
                session.status = status
                session.isStatusSaid = true
            }

        case .sessionContext(let machineId, let sessionId, let context):
            change(SessionKey(machineId: machineId, sessionId: sessionId)) { session in
                session.context = context
                session.isContextSaid = true
            }

        case .sessionLayout(let machineId, let sessionId, let layout):
            change(SessionKey(machineId: machineId, sessionId: sessionId)) { $0.layout = layout }

        case .hud(let machineId, let sessionId, let hud):
            change(SessionKey(machineId: machineId, sessionId: sessionId)) { $0.hud = hud }

        default:
            receivePane(event)
        }
    }

    private func receivePane(_ event: MobileEvent) {
        switch event {
        case .paneDirty(let pane):
            frames[pane]?.dirty()

        case .paneAttached(let pane):
            // a fresh terminal: its last end is behind it, and its screen is read from scratch
            change(pane) { $0.end = nil }
            frames[pane]?.attached()

        case .paneTitle(let pane, let title):
            change(pane) { $0.title = title }

        case .paneLead(let pane, let role, let covered):
            change(pane) { live in
                live.role = role
                live.isCovered = covered
            }

        case .paneEnded(let pane, let end):
            change(pane) { live in
                live.end = end
                live.isCovered = false
            }

        default:
            break
        }
    }

    // a stream failing before it ever showed the session may be failing for a session ended elsewhere: the machine is
    // listed again, and a listing without it lets it go (once per run of failures)
    private func connectionChanged(_ key: SessionKey, _ state: StreamState) {
        let before = sessions[key]
        change(key) { $0.connection = state }
        guard case .failed = state, before?.layout == nil else {
            return
        }
        if case .failed = before?.connection {
            return
        }
        Task {
            await connections.listSessions(machineId: key.machineId)
        }
    }

    private func change(_ key: SessionKey, _ change: (inout LiveSession) -> Void) {
        var session = sessions[key] ?? LiveSession()
        change(&session)
        sessions[key] = session
    }

    private func change(_ pane: PaneRef, _ change: (inout PaneLive) -> Void) {
        self.change(SessionKey(pane)) { session in
            var live = session.panes[pane.paneId] ?? PaneLive()
            change(&live)
            session.panes[pane.paneId] = live
        }
    }

    deinit {
        // nothing to release: the connections hold only weak references back
    }
}
