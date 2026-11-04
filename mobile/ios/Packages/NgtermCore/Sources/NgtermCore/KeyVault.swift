import Foundation

/// The app's private keys, held by the platform (the Keychain): the core loads one for each connection it makes and
/// wipes the bytes once it has used them.
///
/// The core calls it on its actor thread while the actor waits for the answer, so it answers from what it holds
/// (a Keychain read) and never hops synchronously onto the main actor, which may itself be waiting on the core.
public protocol KeyVault: Sendable {
    /// The OpenSSH private key bytes of the key `keyId`.
    ///
    /// - Throws: ``VaultError/Missing(keyId:)`` when no key has that id, ``VaultError/Unavailable(reason:)`` with why
    ///   when it can't be read (the Keychain refused); the connect fails with that reason.
    func load(keyId: String) throws(VaultError) -> Data
}
