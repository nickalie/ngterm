import NgtermCore

/// A machine's listing by family: each session whose parent isn't listed heads one, followed by its companions and
/// theirs.
internal enum SessionFamilies {
    /// `listed` by family, in the listing's order of heads (the core lists parents first, companions after them all).
    ///
    /// A loop of parents has no head: its sessions stand alone, so none goes missing.
    internal static func of(_ listed: [RemoteSession]) -> [[RemoteSession]] {
        let ids = Set(listed.map(\.id))
        var companions: [String: [RemoteSession]] = [:]
        for session in listed {
            if let parent = session.parent, ids.contains(parent) {
                companions[parent, default: []].append(session)
            }
        }
        var placed: Set<String> = []

        func family(_ head: RemoteSession) -> [RemoteSession] {
            guard placed.insert(head.id).inserted else {
                return []
            }
            return [head] + (companions[head.id] ?? []).flatMap(family)
        }

        let heads = listed.filter { session in session.parent.map { !ids.contains($0) } ?? true }.map(family)
        let strays = listed.filter { !placed.contains($0.id) }.map { [$0] }
        return heads + strays
    }
}
