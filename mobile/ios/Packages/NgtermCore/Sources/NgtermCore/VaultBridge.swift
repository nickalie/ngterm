import Foundation
import NgtermCoreBindings

/// The platform's ``KeyVault`` as the core's callback interface.
internal final class VaultBridge: NgtermCoreBindings.KeyVault {
    private let vault: any KeyVault

    internal init(_ vault: any KeyVault) {
        self.vault = vault
    }

    internal func load(keyId: String) throws -> Data {
        try vault.load(keyId: keyId)
    }

    deinit {
        // nothing to release: the vault is the platform's
    }
}
