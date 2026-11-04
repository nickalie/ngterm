import NgtermCore
import Synchronization

/// Host key pins kept in memory, for the tests.
internal final class MemoryHostKeys: HostKeyStore {
    private let stored: Mutex<[String: String]>

    /// The pins, by machine id.
    internal var pins: [String: String] {
        stored.withLock { $0 }
    }

    internal init(_ pins: [String: String]) {
        stored = Mutex(pins)
    }

    internal convenience init() {
        self.init([:])
    }

    internal func pinned(machineId: String) -> String? {
        stored.withLock { $0[machineId] }
    }

    internal func pin(machineId: String, fingerprint: String) {
        stored.withLock { $0[machineId] = fingerprint }
    }

    deinit {
        // nothing to release
    }
}
