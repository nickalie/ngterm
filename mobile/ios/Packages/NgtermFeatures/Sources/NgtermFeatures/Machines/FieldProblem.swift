/// What is wrong with a field of the machine form, in the words the form shows under it.
public enum FieldProblem: CaseIterable, Hashable, Sendable {
    /// `host:port` in the host.
    case hostHasPort
    /// A host with a space in it.
    case hostHasSpace
    /// `user@host` in the host.
    case hostHasUser
    /// No host.
    case hostMissing
    /// No key chosen.
    case keyMissing
    /// A port that isn't 1 to 65535.
    case portInvalid
    /// A user with a space in it.
    case userHasSpace
    /// No user.
    case userMissing

    /// The field it is about.
    public var field: MachineField {
        switch self {
        case .hostHasPort, .hostHasSpace, .hostHasUser, .hostMissing:
            .host

        case .portInvalid:
            .port

        case .userHasSpace, .userMissing:
            .user

        case .keyMissing:
            .key
        }
    }

    /// What the form says under the field: the fix, not the fault.
    public var message: String {
        switch self {
        case .hostMissing:
            String(localized: "Enter the machine's hostname or IP address.")

        case .hostHasSpace:
            String(localized: "A hostname has no spaces.")

        case .hostHasUser:
            String(localized: "Put the user in User, not in the host.")

        case .hostHasPort:
            String(localized: "Put the port in Port, not in the host.")

        case .portInvalid:
            String(localized: "Enter a port from 1 to 65535.")

        case .userMissing:
            String(localized: "Enter the user you sign in as.")

        case .userHasSpace:
            String(localized: "A user name has no spaces.")

        case .keyMissing:
            String(localized: "Choose the key to sign in with.")
        }
    }
}
