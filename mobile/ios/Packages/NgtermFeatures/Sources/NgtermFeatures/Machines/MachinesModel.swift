import Foundation
import NgtermPlatform
import Observation

/// The Machines list and its machine form (mobile/android's `MachinesViewModel`): each machine with its link and
/// sessions, adding, editing and removing machines (with undo), and the questions a connect waits on.
///
/// A change the store can't write leaves the list as it was and says why in ``failure``.
@preconcurrency
@MainActor
@Observable
public final class MachinesModel {
    /// The machine last removed, while its undo is on offer.
    public private(set) var removal: Removal?
    /// Why the last change of the machines failed, until it is dismissed or a change lands.
    public internal(set) var failure: String?
    @ObservationIgnored private let graph: AppGraph

    /// The machines, in the order they were added, each with its link and listed sessions.
    public var items: [MachineItem] {
        graph.machines.machines.map { machine in
            let listing = graph.connections.trees[machine.id]
            return MachineItem(
                machine: machine,
                link: graph.connections.link(of: machine.id),
                isListed: listing != nil,
                sessions: listing ?? []
            )
        }
    }

    /// Whether the device has no network.
    public var isOffline: Bool {
        !graph.network.isOnline
    }

    /// The app's keys, for the form's key choice.
    public var keys: [AppKey] {
        graph.keys.keys
    }

    /// The question a connect waits on that the app asks first: a host key before a sign-in, the first machine first.
    public var prompt: ConnectPrompt? {
        let listed = items
        for item in listed {
            if case .hostKeyUnknown(let fingerprint) = item.link {
                return .hostKey(machine: item.machine, fingerprint: fingerprint)
            }
        }
        for item in listed {
            if case .signIn(let message, let url, _) = item.link {
                return .signIn(machine: item.machine, message: message, url: url)
            }
        }
        return nil
    }

    /// The list over `graph`.
    public init(graph: AppGraph) {
        self.graph = graph
    }

    /// The attached sessions of the machine `machineId`, in its listing's order by family (``AttachedSessions``).
    public func attached(of machineId: String) -> [AttachedSession] {
        graph.attachedSessions.groups.first { $0.machineId == machineId }?.sessions ?? []
    }

    /// The machine `machineId`, if the user has it.
    public func machine(id machineId: String) -> StoredMachine? {
        graph.machines.machine(id: machineId)
    }

    /// The host key pinned for `machineId`, for the form; none when the Keychain refuses.
    public func pin(of machineId: String) -> String? {
        try? graph.pins.fingerprint(machineId: machineId)
    }

    /// Connects (or retries) the machine `machineId`.
    public func connect(machineId: String) {
        if let machine = graph.machines.machine(id: machineId) {
            graph.connections.connect(machine.core)
        }
    }

    /// Closes the connection of the machine `machineId`.
    public func disconnect(machineId: String) {
        graph.connections.disconnect(machineId: machineId)
    }

    /// Saves `draft` as a new machine, or as the machine `editing`, and connects it; false when the draft has
    /// problems or the store couldn't take it (``failure`` says why).
    @discardableResult
    public func save(_ draft: MachineDraft, editing: String?) -> Bool {
        guard let machine = draft.machine(id: editing ?? UUID().uuidString) else {
            return false
        }
        do {
            try graph.machines.save(machine)
        } catch {
            failure = String(localized: "The machine couldn't be saved: \(error.description)")
            return false
        }
        failure = nil
        graph.connections.connect(machine.core)
        return true
    }

    /// Removes the machine `machineId` and its pin, keeping both for ``undoRemove()``.
    public func remove(machineId: String) {
        let machines = graph.machines.machines
        guard let index = machines.firstIndex(where: { $0.id == machineId }) else {
            return
        }
        let pin = pin(of: machineId)
        do {
            try graph.machines.delete(id: machineId)
        } catch {
            failure = String(localized: "The machine couldn't be removed: \(error.description)")
            return
        }
        failure = nil
        graph.connections.disconnect(machineId: machineId)
        // a pin the Keychain keeps is harmless: no other machine has its id (a new one's is a fresh UUID), and Undo
        // sets it again
        try? graph.pins.remove(machineId: machineId)
        removal = Removal(machine: machines[index], index: index, pin: pin)
    }

    /// Puts the machine last removed back, with its pin, and connects it again.
    ///
    /// A machine the store can't put back stays removed, with its undo still on offer, and ``failure`` says why.
    public func undoRemove() {
        guard let undone = removal else {
            return
        }
        do {
            try graph.machines.restore(undone.machine, at: undone.index)
        } catch {
            failure = String(localized: "The machine couldn't be put back: \(error.description)")
            return
        }
        removal = nil
        failure = nil
        // the pin before the connect, which checks the host key against it; one the Keychain won't take back only
        // means the connect asks about the key again
        if let pin = undone.pin {
            try? graph.pins.set(machineId: undone.machine.id, fingerprint: pin)
        }
        graph.connections.connect(undone.machine.core)
    }

    /// The removal's undo is no longer on offer.
    public func forgetRemoval() {
        removal = nil
    }

    /// The failure has been read.
    public func dismissFailure() {
        failure = nil
    }

    /// The user's answer about the unknown host key `fingerprint` of the machine `machineId`.
    public func answerHostKey(machineId: String, fingerprint: String, trust: Bool) {
        graph.connections.answerHostKey(machineId: machineId, fingerprint: fingerprint, trust: trust)
    }

    /// The sheet asking `prompt` went away unanswered (swiped down): Don't connect, if the connect still waits on it.
    public func dismiss(_ prompt: ConnectPrompt) {
        let link = graph.connections.link(of: prompt.machine.id)
        switch (prompt, link) {
        case (.hostKey(let machine, let asked), .hostKeyUnknown(let offered)) where asked == offered:
            answerHostKey(machineId: machine.id, fingerprint: asked, trust: false)

        case (.signIn(let machine, _, _), .signIn):
            disconnect(machineId: machine.id)

        default:
            break
        }
    }

    /// Removes the pin of the machine `machineId` (its host key changed and the user knows why) and connects it
    /// again, which asks about the key it offers now.
    public func removePin(machineId: String) {
        do {
            try graph.pins.remove(machineId: machineId)
        } catch {
            failure = String(localized: "The pinned key couldn't be removed: \(ErrorWords.of(error))")
            return
        }
        graph.connections.disconnect(machineId: machineId)
        connect(machineId: machineId)
    }

    deinit {
        // nothing to release: the graph outlives the list
    }
}
