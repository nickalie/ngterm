import Foundation

/// The sessions the app had attached, by machine, kept as one JSON file in Application Support (`attached.json`).
///
/// iOS usually starts the app afresh for a background refresh, with nothing attached yet, so the refresh reads which
/// sessions to look at from here. The app writes the set whole on every change. A file that no longer decodes reads as
/// none and is kept as `attached.json.bad`; one that can't be read (before the first unlock) reads as none and is left
/// alone until the next change writes it.
@preconcurrency
@MainActor
public final class AttachedStore {
    /// The store's file name.
    public static let fileName = "attached.json"

    /// The sessions as last written: each machine's session ids, by machine id; a machine with none is missing.
    public private(set) var attached: [String: Set<String>]
    private let file: JSONFile<[String: [String]]>

    /// The store kept in `directory`: the app's Application Support unless a test gives its own.
    public init(directory: URL = .applicationSupportDirectory) {
        file = JSONFile(directory: directory, name: Self.fileName)
        let read = (try? file.read()) ?? [:]
        attached = read.compactMapValues { ids in ids.isEmpty ? nil : Set(ids) }
    }

    /// Writes `attached` as the set, leaving out the machines with no session.
    ///
    /// - Throws: ``StoreError/unwritable(reason:)`` when it can't; the set stays as it was.
    public func save(_ attached: [String: Set<String>]) throws(StoreError) {
        let kept = attached.filter { !$0.value.isEmpty }
        try file.write(kept.mapValues { $0.sorted() })
        self.attached = kept
    }

    deinit {
        // nothing to release: the file is written as each change is made
    }
}
