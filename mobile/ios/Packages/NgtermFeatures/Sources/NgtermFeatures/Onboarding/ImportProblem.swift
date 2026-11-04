import NgtermCore

/// Why an import failed, as the import screen says it.
public enum ImportProblem: Hashable, Sendable {
    /// Nothing was pasted or chosen.
    case empty
    /// The key is encrypted: it needs its passphrase.
    case needsPassphrase
    /// The text is not a private key the app can use, for `reason` (the core's words).
    case notAKey(reason: String)
    /// The key was read but could not be kept, for `reason`.
    case notStored(reason: String)
    /// The chosen file could not be read.
    case unreadable
    /// The passphrase given doesn't open the key.
    case wrongPassphrase

    /// Whether it is about the passphrase, so it shows under the passphrase field rather than the key's.
    public var isAboutPassphrase: Bool {
        self == .needsPassphrase || self == .wrongPassphrase
    }

    /// What the screen says.
    public var message: String {
        switch self {
        case .needsPassphrase:
            String(localized: "This key is protected: enter its passphrase.")

        case .wrongPassphrase:
            String(localized: "That passphrase doesn't open this key.")

        case .notAKey(let reason):
            String(localized: "This isn't a private key ngterm can use: \(reason)")

        case .notStored(let reason):
            String(localized: "The key couldn't be stored: \(reason)")

        case .empty:
            String(localized: "Paste a private key or choose its file.")

        case .unreadable:
            String(localized: "That file couldn't be read. Choose another, or paste the key.")
        }
    }

    /// Why the core refused a key.
    public init(_ error: KeyError) {
        switch error {
        case .PassphraseRequired:
            self = .needsPassphrase

        case .WrongPassphrase:
            self = .wrongPassphrase

        case .Invalid(let reason):
            self = .notAKey(reason: reason)
        }
    }
}
