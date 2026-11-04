import Foundation
import NgtermCore
import NgtermFeatures
import Synchronization

/// The core's key functions, faked as mobile/android's `FakeKeyMaker`: ``generate(comment:)`` hands out
/// ``publicKey``; ``importKey(privateKey:passphrase:)`` takes text starting with `-----BEGIN`, wants ``passphrase``
/// when it says ENCRYPTED, and refuses anything else as the core would.
internal final class FakeKeyMaker: KeyMaker {
    /// The public key line it hands out: a made-up key, never a real one.
    internal static let publicKey =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFakeKeyForScreenshotsOnlyNotARealKey0000000000 ngterm@iPhone"
    /// The fake key's fingerprint.
    internal static let fingerprint = "SHA256:q0Gm4d1Xb7pKc2yT9wVZ5eRj8sHnLfA3uMiO6tYvB1E"
    /// The passphrase that opens ``encryptedKey``.
    internal static let passphrase = "correct horse"
    /// A "private key" it reads as encrypted: text in a key's shape, holding nothing.
    internal static let encryptedKey =
        "-----BEGIN OPENSSH PRIVATE KEY-----\nENCRYPTED fake\n-----END OPENSSH PRIVATE KEY-----"

    private let made = Mutex(0)
    private let comments = Mutex<[String]>([])
    private let refusal = Mutex<KeyError?>(nil)

    /// What ``generate(comment:)`` throws instead of making a key, while set.
    internal var generateFailure: KeyError? {
        get { refusal.withLock { $0 } }
        set { refusal.withLock { $0 = newValue } }
    }

    /// How many keys it has made.
    internal var generated: Int {
        made.withLock { $0 }
    }

    /// The comments it was asked to make keys with, in order.
    internal var commented: [String] {
        comments.withLock { $0 }
    }

    internal func generate(comment: String) throws -> KeyPair {
        if let failure = generateFailure {
            throw failure
        }
        let count = made.withLock { count in
            count += 1
            return count
        }
        comments.withLock { $0.append(comment) }
        return KeyPair(
            privateKey: Data("fake private key \(count)".utf8),
            authorizedKey: Self.publicKey,
            fingerprint: Self.fingerprint
        )
    }

    internal func importKey(privateKey: Data, passphrase: String?) throws -> KeyPair {
        let text = String(bytes: privateKey, encoding: .utf8) ?? ""
        guard text.hasPrefix("-----BEGIN") else {
            throw KeyError.Invalid(reason: "unknown key format")
        }
        if text.contains("ENCRYPTED") {
            guard let passphrase else {
                throw KeyError.PassphraseRequired
            }
            guard passphrase == Self.passphrase else {
                throw KeyError.WrongPassphrase
            }
        }
        return KeyPair(
            privateKey: Data([UInt8](privateKey)),
            authorizedKey: Self.publicKey.replacing("ngterm@iPhone", with: "imported"),
            fingerprint: Self.fingerprint
        )
    }

    deinit {
        // nothing to release
    }
}
