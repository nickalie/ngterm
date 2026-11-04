import Foundation
import Security
import Testing

@testable import NgtermPlatform

/// The vault's errors as the core sees them, and the bytes it hands over (the Keychain itself is tested hosted, in
/// NgTermTests).
@Suite("the key vault's errors and bytes")
internal struct KeyVaultErrorTests {
    @Test("a Keychain locked since the device started is unavailable, with that reason")
    internal func locked() {
        let error = KeychainKeyVault.vaultError(for: KeychainError(status: errSecInteractionNotAllowed))
        #expect(error == .Unavailable(reason: "the Keychain stays locked until the device is first unlocked"))
    }

    @Test("any other refusal is unavailable, with the system's words and the status")
    internal func refused() {
        let error = KeychainKeyVault.vaultError(for: KeychainError(status: errSecAuthFailed))
        guard case .Unavailable(let reason) = error else {
            Issue.record("expected unavailable, got \(error)")
            return
        }
        #expect(reason.hasPrefix("the Keychain refused the key: "))
        #expect(reason.hasSuffix("(\(errSecAuthFailed))"))
    }

    @Test("the handed-over bytes are a copy of their own, equal to the stored ones")
    internal func wipingCopy() {
        let stored = Data((0..<600).map { UInt8(truncatingIfNeeded: $0) })
        let copy = Data.wiping(copyOf: stored)
        #expect(copy == stored)
        #expect(Data.wiping(copyOf: Data()).isEmpty)
    }

    @Test("a wiped value is empty")
    internal func wipe() {
        var key = Data("-----BEGIN OPENSSH PRIVATE KEY-----".utf8)
        key.wipe()
        #expect(key.isEmpty)
    }
}
