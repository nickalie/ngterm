import NgtermCore
import Observation

/// A machine's sessions (mobile/android's `SessionsViewModel`): its listing with each session's status and whether
/// this app has it attached, refreshing it, New session (named or not), End session, attaching and detaching.
///
/// The New session sheet's state lives here, so a size change keeps a create under way. ``notice`` carries what the
/// list can't show (an End or an attach that failed, a create whose sheet was closed) for a banner.
@preconcurrency
@MainActor
@Observable
public final class SessionsModel {
    /// The machine's id in the app's store.
    public let machineId: String
    /// Whether a refresh is under way.
    public private(set) var isRefreshing = false
    /// The sessions whose End is under way, by id.
    public private(set) var ending: Set<String> = []
    /// The New session sheet, or `nil` while it is closed.
    public private(set) var newSession: NewSessionState?
    /// What the screen should say in a banner, until it is dismissed.
    public internal(set) var notice: String?
    @ObservationIgnored private let graph: AppGraph
    // which opening of the New session sheet is showing: a create answers only the sheet it began in
    @ObservationIgnored private var sheet = 0
    @ObservationIgnored private var isOpened = false

    /// What the screen knows of the machine; `nil` once it is no longer saved (removed while its screen was open).
    public var state: MachineSessions? {
        guard let machine = graph.machines.machine(id: machineId) else {
            return nil
        }
        let connections = graph.connections
        let listing = connections.trees[machineId]
        return MachineSessions(
            machine: machine,
            link: connections.link(of: machineId),
            isListed: listing != nil,
            sessions: listing ?? [],
            failure: connections.treeFailures[machineId],
            attached: connections.attached[machineId] ?? [],
            isOffline: !graph.network.isOnline,
            isRefreshing: isRefreshing,
            ending: ending
        )
    }

    /// The sessions of the machine `machineId` over `graph`.
    public init(graph: AppGraph, machineId: String) {
        self.graph = graph
        self.machineId = machineId
    }

    /// The screen opened: the machine is listed afresh when it is connected, once per model.
    ///
    /// One that connects or reconnects while the screen is open is listed by ``Connections`` then.
    @discardableResult
    public func opened() -> Task<Void, Never>? {
        guard !isOpened else {
            return nil
        }
        isOpened = true
        guard graph.connections.link(of: machineId) == .connected else {
            return nil
        }
        return Task {
            await graph.connections.listSessions(machineId: machineId)
        }
    }

    /// Lists the sessions again (pull to refresh, or Retry after a failed listing); one under way already is enough.
    public func refresh() async {
        guard !isRefreshing else {
            return
        }
        isRefreshing = true
        defer { isRefreshing = false }
        await graph.connections.listSessions(machineId: machineId)
    }

    /// Connects the machine, or retries it now.
    public func connect() {
        if let machine = graph.machines.machine(id: machineId) {
            graph.connections.connect(machine.core)
        }
    }

    /// Opens the New session sheet.
    public func openNewSession() {
        guard newSession == nil else {
            return
        }
        sheet += 1
        newSession = NewSessionState()
    }

    /// Closes the New session sheet; a create under way goes on, its session shows in the list and its failure in
    /// ``notice``, and a sheet opened after it is not its.
    public func closeNewSession() {
        sheet += 1
        newSession = nil
    }

    /// Creates a session named `name` (blank: the machine names it); the sheet closes once it is listed.
    @discardableResult
    public func create(name: String) -> Task<Void, Never>? {
        guard newSession?.isRunning != true else {
            return nil
        }
        let mine = sheet
        newSession = NewSessionState(isRunning: true)
        return Task {
            do {
                _ = try await graph.connections.createSession(machineId: machineId, name: name)
                if sheet == mine {
                    newSession = nil
                }
            } catch let error as MobileError {
                // a sheet closed meanwhile has no one to tell: the failure goes to the banner
                if sheet == mine {
                    newSession = NewSessionState(failure: error.reason)
                } else {
                    notice = error.reason
                }
            } catch {
                // the task was cancelled: the core still creates it, and its listing shows it
                if sheet == mine {
                    newSession = NewSessionState()
                }
            }
        }
    }

    /// Ends the session `sessionId` there, with no undo (the screen confirms first).
    @discardableResult
    public func end(sessionId: String) -> Task<Void, Never>? {
        guard !ending.contains(sessionId) else {
            return nil
        }
        ending.insert(sessionId)
        return Task {
            defer { ending.remove(sessionId) }
            do {
                try await graph.connections.endSession(machineId: machineId, sessionId: sessionId)
            } catch let error as MobileError {
                notice = error.reason
            } catch {
                // the task was cancelled: the core still ends it, and its listing shows it gone
            }
        }
    }

    /// Attaches the session `sessionId`, then runs `onAttached` (which opens it).
    @discardableResult
    @preconcurrency
    public func attach(sessionId: String, then onAttached: @escaping @MainActor () -> Void) -> Task<Void, Never> {
        Task {
            do {
                try await graph.connections.attach(machineId: machineId, sessionId: sessionId)
                onAttached()
            } catch let error as MobileError {
                notice = error.reason
            } catch {
                // the task was cancelled: nothing opens
            }
        }
    }

    /// Whether the panes of `sessionId` running an agent show as a chat, and the way to set it for that session alone.
    public func chatView(sessionId: String) -> ChatViewChoice {
        let key = SessionKey(machineId: machineId, sessionId: sessionId)
        let chats = graph.chats
        return ChatViewChoice(isOn: chats.isChatView(key)) { isOn in
            chats.setChatView(key, on: isOn)
        }
    }

    /// Detaches the session `sessionId`: it goes on running there.
    public func detach(sessionId: String) {
        graph.connections.detach(machineId: machineId, sessionId: sessionId)
    }

    /// The ``notice`` has been read.
    public func dismissNotice() {
        notice = nil
    }

    deinit {
        // nothing to release: the graph outlives the screen
    }
}
