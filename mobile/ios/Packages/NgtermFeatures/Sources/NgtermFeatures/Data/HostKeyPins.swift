import NgtermCore
import NgtermPlatform

/// The pinned host keys as the app reads and changes them.
///
/// The core's ``HostKeyStore``, plus the app's own reads and changes, which report a refusal. The Keychain's
/// (``KeychainHostKeys``) in the app, memory's (``MemoryHostKeyPins``) in the tests and on the fake core.
public protocol HostKeyPins: HostKeyStore {
    /// The fingerprint pinned for `machineId`, if any.
    func fingerprint(machineId: String) throws -> String?

    /// Pins `fingerprint` for `machineId`, replacing its pin (a removal undone).
    func set(machineId: String, fingerprint: String) throws

    /// Forgets the pin of `machineId`; its next connect asks about whatever key it offers then.
    func remove(machineId: String) throws
}

extension KeychainHostKeys: HostKeyPins {}
