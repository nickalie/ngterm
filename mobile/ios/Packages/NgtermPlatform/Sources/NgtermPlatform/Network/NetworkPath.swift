/// What a network path offers, as ``NetworkMonitor`` compares one with the next.
public struct NetworkPath: Hashable, Sendable {
    /// Whether the path can carry traffic.
    public let isSatisfied: Bool
    /// The names of the interfaces the path may use, the preferred first (`en0`, `pdp_ip0`).
    public let interfaces: [String]

    /// A path that can (`isSatisfied`) or can't carry traffic, over `interfaces`, the preferred first.
    public init(isSatisfied: Bool, interfaces: [String]) {
        self.isSatisfied = isSatisfied
        self.interfaces = interfaces
    }
}
