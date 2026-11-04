import Synchronization

/// Host keys pinned in memory for as long as the process: the tests' ``HostKeyPins``, and the app's on the fake core.
public final class MemoryHostKeyPins: HostKeyPins {
    private let pins = Mutex<[String: String]>([:])

    /// Every pin, by machine id.
    public var all: [String: String] {
        pins.withLock { $0 }
    }

    /// No pins yet.
    public init() {
        // pins come as the user trusts keys
    }

    /// The fingerprint pinned for `machineId`, if any.
    public func pinned(machineId: String) -> String? {
        pins.withLock { $0[machineId] }
    }

    /// Pins `fingerprint` for `machineId`.
    public func pin(machineId: String, fingerprint: String) {
        pins.withLock { $0[machineId] = fingerprint }
    }

    /// The fingerprint pinned for `machineId`, if any.
    public func fingerprint(machineId: String) -> String? {
        pinned(machineId: machineId)
    }

    /// Pins `fingerprint` for `machineId`.
    public func set(machineId: String, fingerprint: String) {
        pin(machineId: machineId, fingerprint: fingerprint)
    }

    /// Forgets the pin of `machineId`.
    public func remove(machineId: String) {
        _ = pins.withLock { $0.removeValue(forKey: machineId) }
    }

    deinit {
        // nothing to release: what it keeps goes with it
    }
}
