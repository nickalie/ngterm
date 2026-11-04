/// What a network path offers, as ``NetworkMonitor`` compares one with the next.
public struct NetworkPath: Hashable, Sendable {
    /// Whether the path can carry traffic.
    public let isSatisfied: Bool
    /// The names of the interfaces the path may use, the preferred first (`en0`, `pdp_ip0`).
    public let interfaces: [String]
    /// The path's gateways, as text: another network behind the same interface (one Wi-Fi to the next) has others.
    public let gateways: Set<String>

    /// A path that can (`isSatisfied`) or can't carry traffic, over `interfaces`, the preferred first, through
    /// `gateways`.
    public init(isSatisfied: Bool, interfaces: [String], gateways: Set<String> = []) {
        self.isSatisfied = isSatisfied
        self.interfaces = interfaces
        self.gateways = gateways
    }
}
