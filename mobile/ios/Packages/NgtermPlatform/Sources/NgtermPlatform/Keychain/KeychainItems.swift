import Foundation
import Security

/// The app's generic-password items of one Keychain service, by account.
///
/// Every item is readable from the device's first unlock on, by this device only
/// (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`: a background refresh with the phone locked can still connect,
/// and neither a backup nor a new device carries it), and is never synchronized through iCloud.
internal struct KeychainItems: Sendable {
    /// The service the items are kept under.
    internal let service: String

    /// Keeps `data` as the item of `account`, replacing what it held.
    internal func set(_ data: Data, account: String) throws(KeychainError) {
        let attributes: [CFString: Any] = [
            kSecValueData: data,
            kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let updated = SecItemUpdate(query(account: account) as CFDictionary, attributes as CFDictionary)
        switch updated {
        case errSecSuccess:
            return

        case errSecItemNotFound:
            let added = SecItemAdd(
                query(account: account).merging(attributes) { _, new in new } as CFDictionary,
                nil
            )
            guard added == errSecSuccess else {
                throw KeychainError(status: added)
            }

        default:
            throw KeychainError(status: updated)
        }
    }

    /// The bytes of the item of `account`; `nil` when there is none.
    internal func data(account: String) throws(KeychainError) -> Data? {
        var lookup = query(account: account)
        lookup[kSecReturnData] = true
        lookup[kSecMatchLimit] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(lookup as CFDictionary, &result)
        switch status {
        case errSecSuccess:
            guard let data = result as? Data else {
                throw KeychainError(status: errSecInternalError)
            }
            return data

        case errSecItemNotFound:
            return nil

        default:
            throw KeychainError(status: status)
        }
    }

    /// Whether `account` has an item, without reading its bytes.
    internal func contains(account: String) throws(KeychainError) -> Bool {
        var lookup = query(account: account)
        lookup[kSecMatchLimit] = kSecMatchLimitOne
        let status = SecItemCopyMatching(lookup as CFDictionary, nil)
        switch status {
        case errSecSuccess:
            return true

        case errSecItemNotFound:
            return false

        default:
            throw KeychainError(status: status)
        }
    }

    /// Removes the item of `account`; nothing happens when there is none.
    internal func remove(account: String) throws(KeychainError) {
        let status = SecItemDelete(query(account: account) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainError(status: status)
        }
    }

    // the item of `account`, never a synchronizable one
    private func query(account: String) -> [CFString: Any] {
        [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: account,
            kSecAttrSynchronizable: false,
            kSecUseDataProtectionKeychain: true,
        ]
    }
}
