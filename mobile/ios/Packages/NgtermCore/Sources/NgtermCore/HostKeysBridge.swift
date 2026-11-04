import NgtermCoreBindings

/// The platform's ``HostKeyStore`` as the core's callback interface.
internal final class HostKeysBridge: NgtermCoreBindings.HostKeyStore {
    private let hosts: any HostKeyStore

    internal init(_ hosts: any HostKeyStore) {
        self.hosts = hosts
    }

    internal func pinned(machineId: String) -> String? {
        hosts.pinned(machineId: machineId)
    }

    internal func pin(machineId: String, fingerprint: String) {
        hosts.pin(machineId: machineId, fingerprint: fingerprint)
    }

    deinit {
        // nothing to release: the store is the platform's
    }
}
