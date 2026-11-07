import Foundation
import NgtermCore
import Observation

/// The app's side of the mobile core's connections (mobile/android's `Connections`): each machine's ``Link``, its last
/// listing and why the last one failed, and the sessions this app has attached.
///
/// An attached session's listing carries the name its stream last said, so a rename on its machine shows everywhere at
/// once, though the machine is listed only now and then.
///
/// It takes the core's ``MobileCoreClient/events`` on the main actor, in the order the core told them, and turns them
/// and its connect answers into ``links``; a connect and a reconnect each list the machine anew. Calls are made on the
/// main actor; the async ones run in tasks of their own, or the caller's.
@preconcurrency
@MainActor
@Observable
public final class Connections {
    private static let millisecondsPerSecond: Double = 1_000

    /// Each machine's link, by id; a machine missing from it is ``Link/offline``.
    public private(set) var links: [String: Link] = [:]
    /// Each connected machine's sessions as it last listed them, by id, its attached sessions under the names their
    /// streams last said.
    public private(set) var trees: [String: [RemoteSession]] = [:]
    /// Why a machine's last listing failed (the core's words), by id, until a listing lands.
    public private(set) var treeFailures: [String: String] = [:]
    /// The sessions this app has attached, by machine id: each machine's session ids.
    public internal(set) var attached: [String: Set<String>] = [:] {
        didSet {
            if attached != oldValue {
                for watcher in attachedWatchers {
                    watcher(attached)
                }
            }
        }
    }

    /// The core, for what this class doesn't wrap.
    @ObservationIgnored public let core: any MobileCoreClient
    /// The clock a reconnect counts down by.
    @ObservationIgnored public let clock: @Sendable () -> Date
    // the record each machine was last connected with: a connect with another replaces the connection
    @ObservationIgnored private var records: [String: Machine] = [:]
    // each machine's listing generation, bumped by every listing that lands and every connection that goes: a listing
    // failure counts only while no newer listing or connection has overtaken the call that failed
    @ObservationIgnored private var generations: [String: UInt64] = [:]
    // the machines whose unknown host key the user declined: the failure their connect reports is no failure to them
    @ObservationIgnored private var declined: Set<String> = []
    // the machines whose connect was cancelled before it listed them: the connection the core makes for them later (a
    // resume's) is listed when it reports Connected; whoever takes a machine out lists it, so it is listed once
    @ObservationIgnored private var unlisted: Set<String> = []
    // the name each attached session's stream last said: it outranks its listing's while it stays attached
    @ObservationIgnored internal var streamNames: [SessionKey: String] = [:]
    // each machine's last listing as the core gave it: trees is it with the stream names over it, so a session that
    // stops being attached shows its listing's name again
    @ObservationIgnored private var listings: [String: [RemoteSession]] = [:]
    @ObservationIgnored private var consumer: Task<Void, Never>?
    // the app's other parts that take the core's events (the live sessions), and those that follow the attached set
    @ObservationIgnored private var listeners: [@MainActor (MobileEvent) -> Void] = []
    @ObservationIgnored private var attachedWatchers: [@MainActor ([String: Set<String>]) -> Void] = []
    /// How many of the core's events have been taken so far (for the tests, which wait for the stream to drain).
    @ObservationIgnored internal private(set) var handledEvents = 0

    /// Whether any machine's connect waits for the user to sign in (``Link/signIn(message:url:isReconnecting:)``).
    public var isSigningIn: Bool {
        links.values.contains { link in
            if case .signIn = link {
                return true
            }
            return false
        }
    }

    /// The connections over `core`, whose events it takes from now on; a reconnect counts down by `clock`.
    @preconcurrency
    public init(core: any MobileCoreClient, clock: @escaping @Sendable () -> Date = { Date() }) {
        self.core = core
        self.clock = clock
        consumer = Task { [weak self, events = core.events] in
            for await event in events {
                guard let self else {
                    return
                }
                receive(event)
            }
        }
    }

    // a machine is reached alike when only its label differs: renaming it keeps its connection
    private static func reachesAlike(_ before: Machine, _ after: Machine) -> Bool {
        var renamed = before
        renamed.label = after.label
        return renamed == after
    }

    // the machine an event about a connection is about: its state, its questions, its listing
    private static func machineId(of event: MobileEvent) -> String? {
        switch event {
        case .machineState(let machineId, _),
            .hostKeyUnknown(let machineId, _),
            .hostKeyChanged(let machineId, _, _),
            .signInRequested(let machineId, _, _),
            .tree(let machineId, _):
            machineId

        default:
            nil
        }
    }

    /// The link of `machineId`.
    public func link(of machineId: String) -> Link {
        links[machineId] ?? .offline
    }

    /// Hands `listener` every event of the core from now on, after this class has taken it, on the main actor; an
    /// event this class drops as stale (about a machine disconnected since) goes to no one.
    @preconcurrency
    public func listen(_ listener: @escaping @MainActor (MobileEvent) -> Void) {
        listeners.append(listener)
    }

    /// Hands `watcher` ``attached`` after each change from now on.
    @preconcurrency
    public func watchAttached(_ watcher: @escaping @MainActor ([String: Set<String>]) -> Void) {
        attachedWatchers.append(watcher)
    }

    /// Connects `machine` (or retries it at once), then lists its sessions for ``trees``.
    ///
    /// A machine already connected as it is stays connected; one connected as it was before an edit connects anew,
    /// and lets go of its attached sessions (the core ends their streams with the old connection). A machine waiting
    /// for the user to sign in keeps waiting (the core joins its connect), unless it was edited: that connect is
    /// overtaken, and the new one asks again. The task it hands back ends once the connect and its listing have.
    @discardableResult
    public func connect(_ machine: Machine) -> Task<Void, Never> {
        let id = machine.id
        let before = records.updateValue(machine, forKey: id)
        let isEdited = before.map { !Self.reachesAlike($0, machine) } ?? false
        if isEdited {
            replaced(id)
        }
        declined.remove(id)
        switch links[id] {
        case .connected, .hostKeyUnknown:
            break

        case .signIn:
            if isEdited {
                links[id] = .connecting
            }

        default:
            links[id] = .connecting
        }
        return Task {
            await connecting(machine)
        }
    }

    /// Lists the sessions of `machineId` again.
    ///
    /// The listing arrives in ``trees``, or why it failed in ``treeFailures``. A listing overtaken by a disconnect, or
    /// by a newer listing or connection, fails nothing.
    public func listSessions(machineId: String) async {
        let generation = generations[machineId, default: 0]
        do {
            _ = try await core.listSessions(machineId: machineId)
        } catch MobileError.Cancelled {
            // a later call reports its own outcome
        } catch MobileError.Stopped {
            links = [:]
        } catch let error as MobileError {
            if generations[machineId, default: 0] == generation, links[machineId] == .connected {
                treeFailures[machineId] = error.reason
            }
        } catch {
            // the caller's task was cancelled: the core still lists, and its tree arrives as an event
        }
    }

    /// The user's answer about the unknown host key `fingerprint` of `machineId`.
    public func answerHostKey(machineId: String, fingerprint: String, trust: Bool) {
        if trust {
            links[machineId] = .connecting
        } else {
            declined.insert(machineId)
            links[machineId] = nil
        }
        guarded { try core.answerHostKey(machineId: machineId, fingerprint: fingerprint, trust: trust) }
    }

    /// Closes the connection of `machineId`, if any, or cancels its connect.
    ///
    /// Its listing and its attached sessions are forgotten (the core ends their streams).
    public func disconnect(machineId: String) {
        guarded { try core.disconnect(machineId: machineId) }
        records[machineId] = nil
        links[machineId] = nil
        bump(machineId)
        listings[machineId] = nil
        trees[machineId] = nil
        treeFailures[machineId] = nil
        attached[machineId] = nil
        streamNames = streamNames.filter { $0.key.machineId != machineId }
    }

    /// `machineId` signs in with the key `keyId` from its next connection attempt, a reconnect included; its
    /// connection and attached sessions stay.
    public func useKey(machineId: String, keyId: String) {
        records[machineId]?.keyId = keyId
        guarded { try core.useKey(machineId: machineId, keyId: keyId) }
    }

    /// The network came back or changed: a machine waiting to reconnect tries at once.
    public func networkChanged() {
        guarded { try core.networkChanged() }
    }

    /// The app has nothing to keep connected: the core closes every connection quietly until ``resume()``, which
    /// connects again the machines that were.
    public func suspend() {
        guarded { try core.suspend() }
    }

    /// Brings back what ``suspend()`` closed.
    public func resume() {
        guarded { try core.resume() }
    }

    /// Takes `event` from the core, in the order the core told it.
    ///
    /// An event about a machine with no record is dropped: it was told before a ``disconnect(machineId:)`` that the
    /// stream delivers after it (a Connected that would show a closed connection, a question about a cancelled
    /// connect). ``connect(_:)`` keeps the record before it calls the core, and a suspend keeps it, so every other
    /// event finds one.
    internal func receive(_ event: MobileEvent) {
        defer { handledEvents += 1 }
        if let machineId = Self.machineId(of: event), records[machineId] == nil {
            return
        }
        switch event {
        case .machineState(let machineId, let state):
            machineState(machineId, state)

        case .hostKeyUnknown(let machineId, let fingerprint):
            links[machineId] = .hostKeyUnknown(fingerprint: fingerprint)

        case .hostKeyChanged(let machineId, let pinned, let offered):
            links[machineId] = .hostKeyChanged(pinned: pinned, offered: offered)

        case .signInRequested(let machineId, let message, let url):
            // a reconnect that has to sign in again (a check that ran out while the connection was down) stays a
            // reconnect, so its Connected lists the machine afresh
            let isReconnecting = links[machineId]?.isReconnect ?? false
            links[machineId] = .signIn(message: message, url: url, isReconnecting: isReconnecting)

        case .tree(let machineId, let sessions):
            listed(machineId, sessions)

        case .sessionName(let machineId, let sessionId, let name):
            named(machineId, sessionId, name)

        default:
            break
        }
        for listener in listeners {
            listener(event)
        }
    }

    private func connecting(_ machine: Machine) async {
        let id = machine.id
        do {
            try await core.connect(machine: machine)
            // connected already: the core reports no state for a machine that didn't change
            if links[id] == .connecting {
                links[id] = .connected
            }
        } catch MobileError.Cancelled {
            // overtaken by a disconnect or another connect, which reports its own state; or set aside by a suspend,
            // whose resume connects it with no caller: that connection is listed when it comes. One that reported
            // Connected already is listed here, unless that event took the marker first and listed it itself
            unlisted.insert(id)
            guard links[id] == .connected, unlisted.remove(id) != nil else {
                return
            }
        } catch let error as MobileError {
            if links[id] == .connecting {
                failed(id, error.reason)
            }
            return
        } catch {
            // the task was cancelled: the core still connects, and reports what becomes of it
            return
        }
        await listSessions(machineId: id)
    }

    private func machineState(_ machineId: String, _ state: MachineState) {
        switch state {
        case .connected:
            reconnected(machineId)

        case .reconnecting(let nextInMs):
            let retryAt = clock().addingTimeInterval(Double(nextInMs) / Self.millisecondsPerSecond)
            links[machineId] = .reconnecting(retryAt: retryAt)

        case .unreachable(let reason):
            failed(machineId, reason)
        }
    }

    // the core restores a lost connection's streams but lists nothing: a machine back from reconnecting is listed
    // afresh, as a listing that failed or was dropped while it was down would otherwise never be redone
    private func reconnected(_ machineId: String) {
        let wasReconnecting = links[machineId]?.isReconnect ?? false
        links[machineId] = .connected
        let wasCancelled = unlisted.remove(machineId) != nil
        if wasReconnecting || wasCancelled {
            Task {
                await listSessions(machineId: machineId)
            }
        }
    }

    // a failure doesn't hide a changed host key (the core reports one with both), and a declined key is no failure
    private func failed(_ machineId: String, _ reason: String) {
        if case .hostKeyChanged = links[machineId] {
            return
        }
        links[machineId] = declined.contains(machineId) ? nil : .unreachable(reason: reason)
    }

    // a listing replaces the last one and its failure; an attached session it no longer has has ended there, so its
    // stream is let go
    private func listed(_ machineId: String, _ sessions: [RemoteSession]) {
        bump(machineId)
        listings[machineId] = sessions
        trees[machineId] = withStreamNames(machineId, sessions)
        treeFailures[machineId] = nil
        let gone = (attached[machineId] ?? []).subtracting(sessions.map(\.id))
        for sessionId in gone.sorted() {
            detach(machineId: machineId, sessionId: sessionId)
        }
    }

    // a stream's word on its session's name (a blank one says nothing); it reaches the listing once the session is
    // attached, as one may land before its attach is recorded
    private func named(_ machineId: String, _ sessionId: String, _ name: String) {
        guard !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return
        }
        streamNames[SessionKey(machineId: machineId, sessionId: sessionId)] = name
        if attached[machineId]?.contains(sessionId) == true {
            renamed(machineId)
        }
    }

    /// Lays the names the attached sessions' streams said over the machine's last listing again.
    internal func renamed(_ machineId: String) {
        guard let listing = listings[machineId] else {
            return
        }
        let named = withStreamNames(machineId, listing)
        if trees[machineId] != named {
            trees[machineId] = named
        }
    }

    // the machine's listing, its attached sessions under the names their streams said
    private func withStreamNames(_ machineId: String, _ sessions: [RemoteSession]) -> [RemoteSession] {
        let attachedHere = attached[machineId] ?? []
        return sessions.map { session in
            guard attachedHere.contains(session.id),
                let said = streamNames[SessionKey(machineId: machineId, sessionId: session.id)], said != session.name
            else {
                return session
            }
            var renamed = session
            renamed.name = said
            return renamed
        }
    }

    // the machine's connection is replaced by one with another record: its attached sessions' streams went with the
    // old one, and a listing call on it no longer counts
    private func replaced(_ machineId: String) {
        bump(machineId)
        for sessionId in (attached[machineId] ?? []).sorted() {
            detach(machineId: machineId, sessionId: sessionId)
        }
    }

    private func bump(_ machineId: String) {
        generations[machineId, default: 0] += 1
    }

    /// Makes a call on the core that has no answer: a core that has stopped answers nothing more, so every machine
    /// shows offline and there is nothing left to tell it.
    internal func guarded(_ call: () throws -> Void) {
        do {
            try call()
        } catch MobileError.Stopped {
            links = [:]
        } catch {
            // a synchronous call throws only Stopped
        }
    }

    isolated deinit {
        consumer?.cancel()
    }
}
