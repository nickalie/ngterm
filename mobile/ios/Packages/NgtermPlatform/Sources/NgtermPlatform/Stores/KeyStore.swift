import Foundation
import Observation

/// What the app shows of its ssh keys, oldest first, kept as one JSON file in Application Support (`keys.json`); the
/// private halves are the Keychain's (``KeychainKeyVault``).
///
/// It reads the file as ``MachineStore`` reads its own: a file that no longer decodes reads as no keys and is kept as
/// `keys.json.bad`, and one that can't be read (or kept) is left alone until a change can read it (or keep it), or
/// ``readIfUnread()`` can; ``readFailure`` says why meanwhile, as ``keys`` is then not what the file holds. A change
/// is written whole and atomically before ``keys`` shows it.
@preconcurrency
@MainActor
@Observable
public final class KeyStore {
    /// The store's file name.
    public static let fileName = "keys.json"

    /// The keys, oldest first.
    public private(set) var keys: [AppKey]
    /// Why the file couldn't be read, while it is there but can't be: ``keys`` is empty then, not what it holds.
    public private(set) var readFailure: StoreError?
    @ObservationIgnored private let file: JSONFile<[AppKey]>

    /// The store kept in `directory`: the app's Application Support unless a test gives its own.
    public init(directory: URL = .applicationSupportDirectory) {
        file = JSONFile(directory: directory, name: Self.fileName)
        do {
            keys = try file.read() ?? []
            readFailure = nil
        } catch {
            keys = []
            readFailure = error
        }
    }

    /// Reads the file again if it couldn't be read before, so its keys show once it can be; a store that has read it
    /// keeps what it has.
    public func readIfUnread() {
        guard readFailure != nil else {
            return
        }
        do {
            keys = try file.read() ?? []
            readFailure = nil
        } catch {
            // still unreadable: no keys, as before, and why it is now
            readFailure = error
        }
    }

    /// The key `id`, if the app has it.
    public func key(id: String) -> AppKey? {
        keys.first { $0.id == id }
    }

    /// Adds `key` after the others, or replaces the one with its id in place.
    public func add(_ key: AppKey) throws(StoreError) {
        if readFailure != nil {
            keys = try file.read() ?? []
            readFailure = nil
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
