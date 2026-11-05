/// The session an alert is about, with the names it says: its machine's and its own.
public struct AlertSubject: Hashable, Sendable {
    /// The session.
    public let key: SessionKey
    /// The machine's name.
    public let machine: String
    /// The session's name.
    public let session: String

    /// The session `key`, named `session` on the machine named `machine`.
    public init(key: SessionKey, machine: String, session: String) {
        self.key = key
        self.machine = machine
        self.session = session
    }
}
