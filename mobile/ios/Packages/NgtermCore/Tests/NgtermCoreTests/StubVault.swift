import Foundation
import NgtermCore

/// A key vault for the tests: it hands over the keys it holds, or refuses every load with `refusal`.
internal final class StubVault: KeyVault {
    private let keys: [String: Data]
    private let refusal: VaultError?

    /// A vault holding no key.
    internal init() {
        keys = [:]
        refusal = nil
    }

    /// A vault holding `keys`, by id.
    internal init(keys: [String: Data]) {
        self.keys = keys
        refusal = nil
    }

    /// A vault refusing every load with `refusal`.
    internal init(refusal: VaultError) {
        keys = [:]
        self.refusal = refusal
    }

    internal func load(keyId: String) throws(VaultError) -> Data {
        if let refusal {
            throw refusal
        }
        guard let key = keys[keyId] else {
            throw VaultError.Missing(keyId: keyId)
        }
        return key
    }

    deinit {
        // nothing to release
    }
}
