import NgtermCore
import NgtermPlatform
import NgtermTerminal
import Observation

/// The clipboard reads of shown panes' programs (Ghostty's `clipboard-read = ask`; mobile/android's `ClipboardReads`).
///
/// Each waits in ``pending`` for the user's Allow, Always or Deny, unless its machine is trusted (``alwaysAllowed``) or
/// the program holds a grant in its terminal, which answer it at once; a read that discloses no data (a listing of the
/// types, or one asking for no type) is answered at once too. The prompt shows what the read would hand over
/// (``preview(_:)``), and its Allow hands over exactly that (Ghostty's macOS form). Every read gets an answer: a read
/// the core dismisses (its pane hidden, its link lost, its program ended: the core denied it) leaves ``pending``, and
/// the app denies the reads of a session it detaches, of a machine whose connection drops, of a pane whose program
/// ends, and all of them before it suspends the core (``denyAll()``).
@preconcurrency
@MainActor
@Observable
public final class ClipboardReads {
    // the most types a listing names (Ghostty's clipboard_response.zig)
    private static let maxListed = 16

    /// The reads waiting for an answer, oldest first: the prompt shows the first.
    public private(set) var pending: [ClipboardRead] = []
    @ObservationIgnored private let connections: Connections
    @ObservationIgnored private let trust: ClipboardTrustStore
    @ObservationIgnored private let clipboard: any TerminalClipboard
    // trusted this run: a machine whose Always the store couldn't write is trusted until the app ends
    private var trustedNow: Set<String> = []
    // revoked this run: a machine whose Ask again the store couldn't write asks until the app ends
    private var revokedNow: Set<String> = []
    // what each read's prompt showed, which its Allow hands over
    @ObservationIgnored private var shown: [ClipboardRead: ClipboardSnapshot] = [:]

    /// The machines whose programs read the clipboard unasked: what Settings lists, and follows as it changes.
    public var alwaysAllowed: Set<String> {
        trust.machines.union(trustedNow).subtracting(revokedNow)
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

    /// What the Allow of `read` would hand over, read once and kept for that Allow.
    public func preview(_ read: ClipboardRead) -> ClipboardSnapshot {
        if let kept = shown[read] {
            return kept
        }
        let snapshot = snapshot(of: read)
        if pending.contains(read) {
            shown[read] = snapshot
        }
        return snapshot
    }

    /// Hands the program of `read` what its prompt showed (what the clipboard holds now, if it showed nothing), this
    /// once; `remember` keeps a grant for the program in its terminal, where the read can keep one.
    public func allow(_ read: ClipboardRead, remember: Bool = false) {
        answer(read, allowed(taken(read), remember: remember && read.canRemember))
    }

    /// Allows `read` as ``allow(_:remember:)`` does, and lets its machine's programs read the clipboard from now on
    /// unasked.
    ///
    /// The machine's other reads already waiting are answered too.
    public func allowAlways(_ read: ClipboardRead, remember: Bool = false) {
        let machine = read.pane.machineId
        trustedNow.insert(machine)
        revokedNow.remove(machine)
        // a machine the store can't keep is trusted until the app ends, and asks again after
        try? trust.allow(machine)
        let same = pending.filter { $0.pane.machineId == machine }
        pending.removeAll { $0.pane.machineId == machine }
        reply(read, allowed(taken(read), remember: remember && read.canRemember))
        for each in same where each != read {
            reply(each, allowed(taken(each), remember: false))
        }
    }

    /// Refuses `read`: its program reads an empty clipboard (OSC 52) or is told it was refused.
    public func deny(_ read: ClipboardRead) {
        shown[read] = nil
        answer(read, .deny)
    }

    /// Refuses every read waiting: the app is about to suspend the core, which closes every connection.
    public func denyAll() {
        deny { _ in true }
    }

    /// Asks again before `machineId`'s programs read the clipboard (Settings' Ask again).
    ///
    /// - Throws: ``StoreError`` when the store can't forget it (its file can't be written, or still can't be read); it
    ///   still asks again until the app ends.
    public func revoke(_ machineId: String) throws(StoreError) {
        trustedNow.remove(machineId)
        revokedNow.insert(machineId)
        try trust.revoke(machineId)
    }

    private func receive(_ event: MobileEvent) {
        switch event {
        case .clipboardReadRequest(
            let pane,
            let readId,
            _,
            let listOnly,
            let mimes,
            let list,
            let granted,
            let canRemember
        ):
            let read = ClipboardRead(
                pane: pane,
                readId: readId,
                listOnly: listOnly,
                mimes: mimes,
                list: list,
                canRemember: canRemember
            )
            let discloses = !listOnly && !mimes.isEmpty
            if !discloses || granted || alwaysAllowed.contains(pane.machineId) {
                reply(read, allowed(snapshot(of: read), remember: false))
            } else {
                pending.append(read)
            }

        case .clipboardReadDismiss(let pane, let readId):
            pending.removeAll { $0.pane == pane && $0.readId == readId }
            shown = shown.filter { $0.key.pane != pane || $0.key.readId != readId }

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
            shown[read] = nil
            reply(read, .deny)
        }
    }

    private func snapshot(of read: ClipboardRead) -> ClipboardSnapshot {
        let asks = !read.listOnly && !read.mimes.isEmpty
        return ClipboardSnapshot(
            contents: asks ? clipboard.read(read.mimes) : [],
            available: read.list ? Array(clipboard.types().prefix(Self.maxListed)) : []
        )
    }

    private func taken(_ read: ClipboardRead) -> ClipboardSnapshot {
        shown.removeValue(forKey: read) ?? snapshot(of: read)
    }

    private func allowed(_ snapshot: ClipboardSnapshot, remember: Bool) -> ClipboardReply {
        .allow(contents: snapshot.contents, available: snapshot.available, remember: remember)
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
