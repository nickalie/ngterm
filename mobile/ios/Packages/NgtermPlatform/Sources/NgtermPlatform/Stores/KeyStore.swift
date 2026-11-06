import Foundation
import Observation

/// What the app shows of its ssh keys, oldest first, kept as one JSON file in Application Support (`keys.json`); the
/// private halves are the Keychain's (``KeychainKeyVault``).
///
/// It reads the file as ``MachineStore`` reads its own: a file that no longer decodes reads as no keys and is kept as
/// `keys.json.bad`, and one that can't be read (or kept) is left alone until a change can read it (or keep it), or
/// ``readIfUnread()`` can. A change is written whole and atomically before ``keys`` shows it.
@preconcurrency
@MainActor
@Observable
public final class KeyStore {
    /// The store's file name.
    public static let fileName = "keys.json"

    /// The keys, oldest first.
    public private(set) var keys: [AppKey]
    @ObservationIgnored private let file: JSONFile<[AppKey]>
    // whether `keys` is what the file holds: false while the file is there but can't be read
    @ObservationIgnored private var loaded: Bool

    /// The store kept in `directory`: the app's Application Support unless a test gives its own.
    public init(directory: URL = .applicationSupportDirectory) {
        file = JSONFile(directory: directory, name: Self.fileName)
        do {
            keys = try file.read() ?? []
            loaded = true
        } catch {
            keys = []
            loaded = false
        }
    }

    /// Reads the file again if it couldn't be read before, so its keys show once it can be; a store that has read it
    /// keeps what it has.
    public func readIfUnread() {
        guard !loaded else {
            return
        }
        do {
            keys = try file.read() ?? []
            loaded = true
        } catch {
            // still unreadable: no keys, as before
        }
    }

    /// The key `id`, if the app has it.
    public func key(id: String) -> AppKey? {
        keys.first { $0.id == id }
    }

    /// Adds `key` after the others, or replaces the one with its id in place.
    public func add(_ key: AppKey) throws(StoreError) {
        if !loaded {
            keys = try file.read() ?? []
            loaded = true
        }
        var next = keys
        if let index = next.firstIndex(where: { $0.id == key.id }) {
            next[index] = key
        } else {
            next.append(key)
        }
        try file.write(next)
        keys = next
    }

    deinit {
        // nothing to release: the file is written as each change is made
    }
}
