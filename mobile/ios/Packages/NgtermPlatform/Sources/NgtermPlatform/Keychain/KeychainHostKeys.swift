import Foundation
import NgtermCore

/// The host keys pinned on first use, one per machine id, each a generic-password item of the Keychain
/// (``KeychainItems`` says how they are kept): the core's ``HostKeyStore``, the one place it learns a machine's pin
/// from.
///
/// The core's ``pinned(machineId:)`` and ``pin(machineId:fingerprint:)`` can't fail, so a Keychain refusal reads as no
/// pin and a pin that can't be kept is dropped (the next connect asks again). The Keychain refuses only before the
/// device's first unlock, when the machines can't be read either, so nothing connects then. The app's own reads and
/// writes, ``fingerprint(machineId:)``, ``set(machineId:fingerprint:)`` and ``remove(machineId:)``, report a refusal.
public struct KeychainHostKeys: HostKeyStore {
    /// The service the app's pins are kept under.
    public static let appService = "com.nickalie.ngterm.ios.hostkeys"

    private let items: KeychainItems

    /// The service the pins are kept under.
    public var service: String {
        items.service
    }

    /// The pins of `service`: the app's unless a test gives its own.
    public init(service: String = Self.appService) {
        items = KeychainItems(service: service)
    }

    /// The fingerprint pinned for `machineId`, if any.
    public func fingerprint(machineId: String) throws(KeychainError) -> String? {
        try items.data(account: machineId).flatMap { String(bytes: $0, encoding: .utf8) }
    }

    /// Pins `fingerprint` for `machineId`, replacing its pin.
    public func set(machineId: String, fingerprint: String) throws(KeychainError) {
        try items.set(Data(fingerprint.utf8), account: machineId)
    }

    /// Forgets the pin of `machineId` (Remove pinned key, or the machine removed).
    ///
    /// Its next connect asks about whatever key it offers then. Nothing happens when it has none.
    public func remove(machineId: String) throws(KeychainError) {
        try items.remove(account: machineId)
    }

    /// The fingerprint pinned for `machineId`; none when the Keychain refuses.
    public func pinned(machineId: String) -> String? {
        try? fingerprint(machineId: machineId)
    }

    /// Pins `fingerprint` for `machineId`, as the user accepted it; dropped when the Keychain refuses.
    public func pin(machineId: String, fingerprint: String) {
        try? set(machineId: machineId, fingerprint: fingerprint)
    }
}
