import Foundation
import NgtermCore
import Observation

/// The app's side of the mobile core's connections (mobile/android's `Connections`): each machine's ``Link``, its last
/// listing and why the last one failed, and the sessions this app has attached.
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
    /// Each connected machine's sessions as it last listed them, by id.
    public private(set) var trees: [String: [RemoteSession]] = [:]
    /// Why a machine's last listing failed (the core's words), by id, until a listing lands.
    public private(set) var treeFailures: [String: String] = [:]
    /// The sessions this app has attached, by machine id: each machine's session ids.
    public internal(set) var attached: [String: Set<String>] = [:]

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
    @ObservationIgnored private var consumer: Task<Void, Never>?
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

    /// The link of `machineId`.
    public func link(of machineId: String) -> Link {
        links[machineId] ?? .offline
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
        trees[machineId] = nil
        treeFailures[machineId] = nil
        attached[machineId] = nil
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
    internal func receive(_ event: MobileEvent) {
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

        default:
            break
        }
        handledEvents += 1
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
        trees[machineId] = sessions
        treeFailures[machineId] = nil
        let gone = (attached[machineId] ?? []).subtracting(sessions.map(\.id))
        for sessionId in gone.sorted() {
            detach(machineId: machineId, sessionId: sessionId)
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
