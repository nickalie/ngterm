import Foundation
import NgtermCore

/// The core's key functions (``CoreKeys``): the app's ``KeyMaker``.
public struct CoreKeyMaker: KeyMaker {
    /// The core's key functions.
    public init() {
        // nothing to hold: the functions are the core's
    }

    /// A new ed25519 key from the core.
    public func generate(comment: String) throws -> KeyPair {
        try CoreKeys.generate(comment: comment)
    }

    /// The key the user brought, read by the core.
    public func importKey(privateKey: Data, passphrase: String?) throws -> KeyPair {
        try CoreKeys.importKey(privateKey: privateKey, passphrase: passphrase)
    }
}
