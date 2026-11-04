/// A field of the machine form.
public enum MachineField: CaseIterable, Hashable, Sendable {
    /// Its host name or address.
    case host
    /// The key it signs in with.
    case key
    /// The machine's name.
    case label
    /// Its ssh port.
    case port
    /// The user it signs in as.
    case user
}
