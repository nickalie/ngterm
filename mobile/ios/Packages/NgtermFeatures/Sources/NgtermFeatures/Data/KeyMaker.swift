import Foundation
import NgtermCore

/// Makes and reads ssh keys: the core's (``CoreKeyMaker``) in the app, a fake in its tests.
///
/// ``AppKeys`` calls it off the main actor: reading a key with a passphrase takes a while on purpose.
public protocol KeyMaker: Sendable {
    /// A new ed25519 key, its public half commented `comment`.
    ///
    /// - Throws: ``KeyError/Invalid(reason:)`` when the key can't be made.
    func generate(comment: String) throws -> KeyPair

    /// The key in `privateKey`, opened with `passphrase` when it is encrypted.
    ///
    /// - Throws: ``KeyError/PassphraseRequired``, ``KeyError/WrongPassphrase`` or ``KeyError/Invalid(reason:)``.
    func importKey(privateKey: Data, passphrase: String?) throws -> KeyPair
}
