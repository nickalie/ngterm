import Foundation
import Observation

/// The machines whose programs may read the clipboard without asking ("Always allow on build-01"), by machine id.
///
/// They are kept as one JSON file in Application Support (`clipboard-trust.json`; mobile/android's `ClipboardTrust`).
/// Settings lists and revokes them.
///
/// It reads the file as ``MachineStore`` reads its own: one that no longer decodes reads as none and is kept as
/// `clipboard-trust.json.bad`. One that can't be read (before the device's first unlock), or a corrupt one whose copy
/// can't be written, reads as none too, so every read asks (the safe side), but is left alone: the next change reads it
/// again, and fails rather than write over the machines it holds while it still can't be read. A change is written
/// whole before ``machines`` shows it.
@preconcurrency
@MainActor
@Observable
public final class ClipboardTrustStore {
    /// The store's file name.
    public static let fileName = "clipboard-trust.json"

    /// The trusted machines' ids, as last written.
    public private(set) var machines: Set<String>
    @ObservationIgnored private let file: JSONFile<[String]>
    // whether `machines` is what the file holds: false while the file is there but can't be read
    @ObservationIgnored private var loaded: Bool

    /// The store kept in `directory`: the app's Application Support unless a test gives its own.
    public init(directory: URL = .applicationSupportDirectory) {
        file = JSONFile(directory: directory, name: Self.fileName)
        do {
            machines = Set(try file.read() ?? [])
            loaded = true
        } catch {
            machines = []
            loaded = false
        }
    }

    /// Lets the programs on `machineId` read the clipboard without asking.
    ///
    /// - Throws: ``StoreError/unwritable(reason:)`` when it can't write the change, or
    ///   ``StoreError/unreadable(reason:)`` when the file is there but still can't be read; the machines stay as they
    ///   were.
    public func allow(_ machineId: String) throws(StoreError) {
        try change { $0.insert(machineId) }
    }

    /// Asks again before the programs on `machineId` read the clipboard.
    ///
    /// - Throws: ``StoreError/unwritable(reason:)`` when it can't write the change, or
    ///   ``StoreError/unreadable(reason:)`` when the file is there but still can't be read; the machines stay as they
    ///   were.
    public func revoke(_ machineId: String) throws(StoreError) {
        try change { $0.remove(machineId) }
    }

    // applies `edit` to the machines and writes the result, then shows it; a file that couldn't be read is read first
    private func change(_ edit: (inout Set<String>) -> Void) throws(StoreError) {
        if !loaded {
            machines = Set(try file.read() ?? [])
            loaded = true
        }
        var next = machines
        edit(&next)
        try file.write(next.sorted())
        machines = next
    }

    deinit {
        // nothing to release: the file is written as each change is made
    }
}
