import NgtermCore
import NgtermPlatform
import NgtermTerminal
import Observation

/// The clipboard reads of shown panes' programs (Ghostty's `clipboard-read = ask`; mobile/android's `ClipboardReads`).
///
/// Each waits in ``pending`` for the user's Allow, Always or Deny, unless its machine is trusted (``alwaysAllowed``),
/// which answers it at once. Every read gets an answer: a read the core dismisses (its pane hidden, its link lost, its
/// program ended: the core denied it) leaves ``pending``, and the app denies the reads of a session it detaches, of a
/// machine whose connection drops, of a pane whose program ends, and all of them before it suspends the core
/// (``denyAll()``).
@preconcurrency
@MainActor
@Observable
public final class ClipboardReads {
    /// The reads waiting for an answer, oldest first: the prompt shows the first.
    public private(set) var pending: [ClipboardRead] = []
    @ObservationIgnored private let connections: Connections
    @ObservationIgnored private let trust: ClipboardTrustStore
    @ObservationIgnored private let clipboard: any TerminalClipboard
    // trusted this run: a machine whose Always the store couldn't write is trusted until the app ends
    @ObservationIgnored private var trustedNow: Set<String> = []

    /// The machines whose programs read the clipboard unasked: what Settings lists.
    public var alwaysAllowed: Set<String> {
        trust.machines.union(trustedNow)
    }

    /// The reads over `connections`, whose events they take from now on, trusting the machines in `trust` and handing
    /// over what `clipboard` holds.
    public init(connections: Connections, trust: ClipboardTrustStore, clipboard: any TerminalClipboard) {
        self.connections = connections
        self.trust = trust
        self.clipboard = clipboard
        connections.listen { [weak self] event in
            self?.receive(event)
        }
        connections.watchAttached { [weak self] attached in
            self?.deny { attached[$0.pane.machineId]?.contains($0.pane.sessionId) != true }
        }
    }

    /// Hands the program of `read` the clipboard's text, this once.
    public func allow(_ read: ClipboardRead) {
        answer(read, .allow(text: clipboard.text()))
    }

    /// Hands the program of `read` the clipboard's text, and lets its machine's programs read it from now on unasked.
    ///
    /// The machine's other reads already waiting are answered with it.
    public func allowAlways(_ read: ClipboardRead) {
        let machine = read.pane.machineId
        trustedNow.insert(machine)
        // a machine the store can't keep is trusted until the app ends, and asks again after
        try? trust.allow(machine)
        let same = pending.filter { $0.pane.machineId == machine }
        pending.removeAll { $0.pane.machineId == machine }
        let text = clipboard.text()
        for each in [read] + same.filter({ $0 != read }) {
            reply(each, .allow(text: text))
        }
    }

    /// Refuses `read`: its program reads an empty clipboard.
    public func deny(_ read: ClipboardRead) {
        answer(read, .deny)
    }

    /// Refuses every read waiting: the app is about to suspend the core, which closes every connection.
    public func denyAll() {
        deny { _ in true }
    }

    /// Asks again before `machineId`'s programs read the clipboard (Settings' Ask again).
    ///
    /// - Throws: ``StoreError/unwritable(reason:)`` when the store can't forget it; it still asks again until the app
    ///   ends.
    public func revoke(_ machineId: String) throws(StoreError) {
        trustedNow.remove(machineId)
        try trust.revoke(machineId)
    }

    private func receive(_ event: MobileEvent) {
        switch event {
        case .clipboardReadRequest(let pane, let readId, _, let listOnly):
            let read = ClipboardRead(pane: pane, readId: readId, listOnly: listOnly)
            if alwaysAllowed.contains(pane.machineId) {
                reply(read, .allow(text: clipboard.text()))
            } else {
                pending.append(read)
            }

        case .clipboardReadDismiss(let pane, let readId):
            pending.removeAll { $0.pane == pane && $0.readId == readId }

        case .paneEnded(let pane, _):
            deny { $0.pane == pane }

        case .machineState(let machineId, let state):
            if case .connected = state {
                return
            }
            deny { $0.pane.machineId == machineId }

        default:
            break
        }
    }

    // denies every read waiting that `matches`
    private func deny(where matches: (ClipboardRead) -> Bool) {
        let denied = pending.filter(matches)
        guard !denied.isEmpty else {
            return
        }
        pending.removeAll(where: matches)
        for read in denied {
            reply(read, .deny)
        }
    }

    private func answer(_ read: ClipboardRead, _ reply: ClipboardReply) {
        pending.removeAll { $0 == read }
        self.reply(read, reply)
    }

    private func reply(_ read: ClipboardRead, _ reply: ClipboardReply) {
        let pane = read.pane
        // a core that has stopped leaves no program waiting
        try? connections.core.answerClipboardRead(
            machineId: pane.machineId,
            sessionId: pane.sessionId,
            paneId: pane.paneId,
            readId: read.readId,
            reply: reply
        )
    }

    deinit {
        // nothing to release: the connections hold only weak references back
    }
}
