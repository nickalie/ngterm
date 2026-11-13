import NgtermCore
import NgtermPlatform

/// The attached sessions across machines, as the session view, its moves and the sidebar show them (mobile/android's
/// `SessionViewModel` lists).
///
/// Each machine's attached sessions come in its listing's order by family (a parent, then its companions under it, as
/// a machine's sessions list them), any it no longer lists after them; a machine with none is left out. A companion
/// whose parent is attached too is nested under it. It reads the stores and the routers as they are now (all
/// observable), and remembers the name each session was last shown with, for the gone state once its machine's listing
/// has dropped it.
@preconcurrency
@MainActor
public final class AttachedSessions {
    private let machines: MachineStore
    private let connections: Connections
    private let live: LiveSessions
    // the last state each session was shown with (main actor), so a session gone from its listing keeps its name
    private var seen: [SessionKey: AttachedSession] = [:]

    /// Every machine's attached sessions, in the store's machine order.
    public var groups: [AttachedGroup] {
        machines.machines.compactMap(group)
    }

    /// Every attached session, in the order the lists show them.
    public var sessions: [AttachedSession] {
        groups.flatMap(\.sessions)
    }

    /// The attached sessions over the stored `machines`, their `connections` and their `live` streams.
    public init(machines: MachineStore, connections: Connections, live: LiveSessions) {
        self.machines = machines
        self.connections = connections
        self.live = live
    }

    // the stream's word wins once it has said one, a clear included; the listing's stands in until then
    private static func status(stream: LiveSession, listing: RemoteSession?) -> AgentStatus? {
        if stream.isStatusSaid {
            return stream.status.map { AgentStatus(text: $0.status, color: $0.color, shape: $0.shape) }
        }
        guard let listing, let status = listing.status.flatMap(unlessBlank) else {
            return nil
        }
        return AgentStatus(text: status, color: listing.statusColor, shape: listing.statusShape)
    }

    private static func unlessBlank(_ text: String) -> String? {
        text.allSatisfy(\.isWhitespace) ? nil : text
    }

    /// The attached session `key`, as the lists show it now; `nil` when it isn't attached or its machine is gone.
    public func session(_ key: SessionKey) -> AttachedSession? {
        guard let machine = machines.machine(id: key.machineId),
            connections.attached[key.machineId]?.contains(key.sessionId) == true
        else {
            return nil
        }
        return attached(key, of: machine)
    }

    /// The session `key` as it was last shown, attached or not since.
    public func lastSeen(_ key: SessionKey) -> AttachedSession? {
        seen[key]
    }

    /// The session after (or before) `key` in the lists' order, wrapping around; `nil` with fewer than two.
    public func neighbor(of key: SessionKey, forward: Bool) -> SessionKey? {
        sessions.map(\.key).cyclicNeighbor(of: key, forward: forward)
    }

    private func group(_ machine: StoredMachine) -> AttachedGroup? {
        let ids = connections.attached[machine.id] ?? []
        guard !ids.isEmpty else {
            return nil
        }
        let tree = connections.trees[machine.id] ?? []
        let names = Dictionary(tree.map { ($0.id, $0.name) }) { first, _ in first }
        let listed = SessionFamilies.of(tree).joined().filter { ids.contains($0.id) }.map { ($0.id, $0.parent) }
        let unlisted = ids.subtracting(names.keys).sorted().map { ($0, String?.none) }
        // the names of the sessions placed so far: a companion nests only under a parent shown above it
        var shown: [String: String] = [:]
        let members = (listed + unlisted).map { id, parent in
            let session = attached(
                SessionKey(machineId: machine.id, sessionId: id),
                of: machine,
                companionOf: parent.map { shown[$0] ?? names[$0] ?? $0 },
                isNested: parent.map { shown[$0] != nil } ?? false
            )
            shown[id] = session.name
            return session
        }
        return AttachedGroup(machineId: machine.id, machine: machine.title, sessions: members)
    }

    private func attached(
        _ key: SessionKey,
        of machine: StoredMachine,
        companionOf: String? = nil,
        isNested: Bool = false
    ) -> AttachedSession {
        let listing = connections.trees[key.machineId]?.first { $0.id == key.sessionId }
        let stream = live.sessions[key] ?? LiveSession()
        var made = AttachedSession(
            key: key,
            machine: machine.title,
            name: listing?.name ?? seen[key]?.name ?? key.sessionId,
            link: connections.link(of: key.machineId),
            live: stream,
            context: stream.isContextSaid ? stream.context : listing?.context.flatMap(Self.unlessBlank),
            status: Self.status(stream: stream, listing: listing),
            companionOf: companionOf,
            isNested: isNested
        )
        made.peer = connections.peers[key.machineId]
        seen[key] = made
        return made
    }

    deinit {
        // nothing to release
    }
}
