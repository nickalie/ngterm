import NgtermPlatform

/// A machine whose programs read the clipboard without asking, as Settings lists it.
public struct TrustedMachine: Hashable, Sendable, Identifiable {
    /// The machine's id.
    public let id: String
    /// Its name.
    public let title: String
    /// `user@host:port`.
    public let address: String
    /// Its address in words, for VoiceOver.
    public let spokenAddress: String

    /// The machine `machine`.
    public init(_ machine: StoredMachine) {
        id = machine.id
        title = machine.title
        address = machine.address
        spokenAddress = machine.spokenAddress
    }
}
