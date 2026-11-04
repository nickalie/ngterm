import Foundation
import NgtermCoreBindings

/// The core's key functions.
///
/// The app's keys are made and imported in Rust, which hands back the OpenSSH form the ``KeyVault`` keeps.
public enum CoreKeys {
    /// A new ed25519 key, its public half commented `comment` (shown in the machine's `authorized_keys`).
    ///
    /// - Throws: ``KeyError/Invalid(reason:)`` when the key can't be made.
    public static func generate(comment: String) throws -> KeyPair {
        try generateKey(comment: comment)
    }

    /// A key the user imports, as the unencrypted OpenSSH key the app keeps.
    ///
    /// The key may be OpenSSH, PKCS#1 or PKCS#8 PEM, opened with `passphrase` when it is encrypted. The core wipes its
    /// copies of the bytes and the passphrase once read.
    ///
    /// - Throws: ``KeyError/PassphraseRequired``, ``KeyError/WrongPassphrase`` or ``KeyError/Invalid(reason:)``.
    public static func importKey(privateKey: Data, passphrase: String?) throws -> KeyPair {
        try NgtermCoreBindings.importKey(privateKey: privateKey, passphrase: passphrase)
    }
}
