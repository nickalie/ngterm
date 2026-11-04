import Foundation
import Synchronization

/// Private keys kept in memory for as long as the process: the tests' ``PrivateKeys``, and the app's on the fake core,
/// which never loads one.
public final class MemoryPrivateKeys: PrivateKeys {
    private let kept = Mutex<[String: Data]>([:])

    /// The ids of the keys kept.
    public var keyIds: Set<String> {
        kept.withLock { Set($0.keys) }
    }

    /// No keys yet.
    public init() {
        // keys come through store(keyId:privateKey:)
    }

    /// The private key `keyId`, if it is kept.
    public func privateKey(keyId: String) -> Data? {
        kept.withLock { $0[keyId] }
    }

    /// Keeps a copy of `privateKey` as `keyId`, then zeroes `privateKey`.
    public func store(keyId: String, privateKey: inout Data) {
        // bytes of its own, not a share of the buffer about to be zeroed
        let copy = Data([UInt8](privateKey))
        kept.withLock { $0[keyId] = copy }
        privateKey.resetBytes(in: privateKey.startIndex..<privateKey.endIndex)
    }

    /// Forgets `keyId`.
    public func delete(keyId: String) {
        _ = kept.withLock { $0.removeValue(forKey: keyId) }
    }

    deinit {
        // nothing to release: what it keeps goes with it
    }
}
