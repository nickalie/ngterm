import NgtermPlatform

/// One of the app's keys as Settings lists it (mobile/android's `KeyItem`): the machines that sign in with it, and
/// whether that is every one.
public struct KeyItem: Hashable, Sendable, Identifiable {
    /// The key.
    public let key: AppKey
    /// The names of the machines that use it, in the machines' order.
    public let usedBy: [String]
    /// Whether every machine uses it (so with no machine at all).
    public let isEveryMachine: Bool

    /// The key's id.
    public var id: String {
        key.id
    }

    /// `key` as `machines` use it.
    public init(key: AppKey, machines: [StoredMachine]) {
        self.key = key
        let users = machines.filter { $0.keyId == key.id }
        usedBy = users.map(\.title)
        isEveryMachine = users.count == machines.count
    }
}
