import Security

/// The Keychain refused a call: its status, and the system's words for it.
public struct KeychainError: Error, Equatable, Sendable, CustomStringConvertible {
    /// The Keychain's status code (`errSec...`).
    public let status: OSStatus

    /// Whether the Keychain refused because the device has not been unlocked since it started.
    public var isLocked: Bool {
        status == errSecInteractionNotAllowed
    }

    /// The system's words for the status, with its code.
    public var description: String {
        let words = SecCopyErrorMessageString(status, nil).map { $0 as String } ?? "unknown error"
        return "\(words) (\(status))"
    }

    /// The Keychain's refusal with `status`.
    public init(status: OSStatus) {
        self.status = status
    }
}
