import NgtermPlatform

/// A question a machine's connect waits on, which the app asks in a sheet: a host key to trust, or a sign-in.
public enum ConnectPrompt: Hashable, Sendable, Identifiable {
    /// `machine` offered `fingerprint` and has no pin: trust it or not.
    case hostKey(machine: StoredMachine, fingerprint: String)
    /// `machine` asks the user to sign in at `url`, with its `message`.
    case signIn(machine: StoredMachine, message: String, url: String)

    /// The machine it is about.
    public var machine: StoredMachine {
        switch self {
        case .hostKey(let machine, _), .signIn(let machine, _, _):
            machine
        }
    }

    /// One prompt per machine and question: a new question for the same machine is a new sheet.
    public var id: String {
        switch self {
        case .hostKey(let machine, let fingerprint):
            "host-key \(machine.id) \(fingerprint)"

        case .signIn(let machine, _, let url):
            "sign-in \(machine.id) \(url)"
        }
    }
}
