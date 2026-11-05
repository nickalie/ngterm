import Foundation

/// The machines whose programs may read the clipboard without asking ("Always allow on build-01"), by machine id.
///
/// They are kept as one JSON file in Application Support (`clipboard-trust.json`; mobile/android's `ClipboardTrust`).
/// Settings lists and revokes them.
///
/// It reads the file as ``AttachedStore`` reads its own: one that no longer decodes reads as none and is kept as
/// `clipboard-trust.json.bad`, and one that can't be read (before the first unlock) reads as none, so every read asks
/// (the safe side). A change is written whole before ``machines`` shows it.
@preconcurrency
@MainActor
public final class ClipboardTrustStore {
    /// The store's file name.
    public static let fileName = "clipboard-trust.json"

    /// The trusted machines' ids, as last written.
    public private(set) var machines: Set<String>
    private let file: JSONFile<[String]>

    /// The store kept in `directory`: the app's Application Support unless a test gives its own.
    public init(directory: URL = .applicationSupportDirectory) {
        file = JSONFile(directory: directory, name: Self.fileName)
        machines = Set((try? file.read()) ?? [])
    }

    /// Lets the programs on `machineId` read the clipboard without asking.
    ///
    /// - Throws: ``StoreError/unwritable(reason:)`` when it can't; the machines stay as they were.
    public func allow(_ machineId: String) throws(StoreError) {
        try save(machines.union([machineId]))
    }

    /// Asks again before the programs on `machineId` read the clipboard.
    ///
    /// - Throws: ``StoreError/unwritable(reason:)`` when it can't; the machines stay as they were.
    public func revoke(_ machineId: String) throws(StoreError) {
        try save(machines.subtracting([machineId]))
    }

    private func save(_ next: Set<String>) throws(StoreError) {
        try file.write(next.sorted())
        machines = next
    }

    deinit {
        // nothing to release: the file is written as each change is made
    }
}
