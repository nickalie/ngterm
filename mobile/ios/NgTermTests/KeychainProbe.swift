import Foundation
import Security

/// The Keychain items of a test's own service, read as the system keeps them: their attributes, and their removal.
internal struct KeychainProbe {
    /// The service, unique to the test.
    internal let service = "com.nickalie.ngterm.ios.tests.\(UUID().uuidString)"

    /// The attributes of the item of `account`, synchronizable or not; none when there is no item.
    internal func attributes(account: String) -> [String: Any] {
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: account,
            kSecAttrSynchronizable: kSecAttrSynchronizableAny,
            kSecReturnAttributes: true,
            kSecMatchLimit: kSecMatchLimitOne,
        ]
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess else {
            return [:]
        }
        return result as? [String: Any] ?? [:]
    }

    /// How many items the service holds, synchronizable or not.
    internal func count() -> Int {
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrSynchronizable: kSecAttrSynchronizableAny,
            kSecReturnAttributes: true,
            kSecMatchLimit: kSecMatchLimitAll,
        ]
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess else {
            return 0
        }
        return (result as? [[String: Any]])?.count ?? 0
    }

    /// Removes every item of the service.
    internal func removeAll() {
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrSynchronizable: kSecAttrSynchronizableAny,
        ]
        SecItemDelete(query as CFDictionary)
    }
}
