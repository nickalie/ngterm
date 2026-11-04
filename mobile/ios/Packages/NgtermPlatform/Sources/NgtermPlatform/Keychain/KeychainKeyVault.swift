import Foundation
import NgtermCore

/// The app's private keys, each a generic-password item of the Keychain (``KeychainItems`` says how they are kept):
/// the core's ``KeyVault``.
///
/// ``load(keyId:)`` hands one over for a connection, in memory of its own that is zeroed once the core has taken its
/// copy (which the core wipes once used); ``store(keyId:privateKey:)`` wipes what it is handed. The Keychain's own
/// buffers are the system's.
public struct KeychainKeyVault: KeyVault {
    /// The service the app's keys are kept under.
    public static let appService = "com.nickalie.ngterm.ios.keys"

    private let items: KeychainItems

    /// The service the keys are kept under.
    public var service: String {
        items.service
    }

    /// The vault over the keys of `service`: the app's unless a test gives its own.
    public init(service: String = Self.appService) {
        items = KeychainItems(service: service)
    }

    /// The core's error for the Keychain's refusal `error` to hand over a key.
    internal static func vaultError(for error: KeychainError) -> VaultError {
        if error.isLocked {
            return .Unavailable(reason: "the Keychain stays locked until the device is first unlocked")
        }
        return .Unavailable(reason: "the Keychain refused the key: \(error)")
    }

    /// Keeps `privateKey` (OpenSSH form) as `keyId`, replacing the key it held, then wipes `privateKey`, whether it
    /// was kept or not.
    public func store(keyId: String, privateKey: inout Data) throws(KeychainError) {
        defer {
            privateKey.wipe()
        }
        try items.set(privateKey, account: keyId)
    }

    /// Whether the vault holds `keyId`.
    public func has(keyId: String) throws(KeychainError) -> Bool {
        try items.contains(account: keyId)
    }

    /// Forgets `keyId`; nothing happens when the vault doesn't hold it.
    public func delete(keyId: String) throws(KeychainError) {
        try items.remove(account: keyId)
    }

    /// The OpenSSH private key bytes of `keyId`, in memory that is zeroed once they are let go.
    ///
    /// - Throws: ``VaultError/Missing(keyId:)`` when the vault doesn't hold it, ``VaultError/Unavailable(reason:)``
    ///   when the Keychain refused (still locked since the device started, say).
    public func load(keyId: String) throws(VaultError) -> Data {
        let stored: Data?
        do {
            stored = try items.data(account: keyId)
        } catch {
            throw Self.vaultError(for: error)
        }
        guard let stored else {
            throw .Missing(keyId: keyId)
        }
        return .wiping(copyOf: stored)
    }
}
