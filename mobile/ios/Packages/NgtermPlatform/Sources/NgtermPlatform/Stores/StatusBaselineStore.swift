import Foundation

/// The last agent status the app saw of each attached session, by machine id and then session id, kept as one JSON
/// file in Application Support (`status-baseline.json`).
///
/// A background refresh, which iOS usually runs in a process started afresh, compares each listed status with it, so a
/// turn to waiting or done notifies once. A file that no longer decodes reads as none and is kept as
/// `status-baseline.json.bad`; one that can't be read (before the first unlock) reads as none and is left alone until
/// the next change writes it.
@preconcurrency
@MainActor
public final class StatusBaselineStore {
    /// The store's file name.
    public static let fileName = "status-baseline.json"

    /// The statuses as last written: each machine's sessions' statuses (their wire names), by machine id and then
    /// session id; a session with no status is missing, and so is a machine with none.
    public private(set) var statuses: [String: [String: String]]
    private let file: JSONFile<[String: [String: String]]>

    /// The store kept in `directory`: the app's Application Support unless a test gives its own.
    public init(directory: URL = .applicationSupportDirectory) {
        file = JSONFile(directory: directory, name: Self.fileName)
        let read = (try? file.read()) ?? [:]
        statuses = read.filter { !$0.value.isEmpty }
    }

    /// Writes `statuses` whole, leaving out the machines with none.
    ///
    /// - Throws: ``StoreError/unwritable(reason:)`` when it can't; the statuses stay as they were.
    public func save(_ statuses: [String: [String: String]]) throws(StoreError) {
        let kept = statuses.filter { !$0.value.isEmpty }
        try file.write(kept)
        self.statuses = kept
    }

    deinit {
        // nothing to release: the file is written as each change is made
    }
}
