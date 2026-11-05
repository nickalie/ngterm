import Foundation
import NgtermCoreFakes
import NgtermPlatform

@testable import NgtermFeatures

/// The app's graph over fakes, as mobile/android's `TestGraph`: the core a ``FakeMobileCore`` pinning in memory, the
/// stores real ones in a directory of the test's own, the keys made by a ``FakeKeyMaker`` and kept in memory, the
/// network a ``TestPathSource``, the clock a ``TestClock`` and the clipboard a ``MemoryClipboard``. ``remove()``
/// deletes the directory.
@MainActor
internal final class TestGraph {
    internal let directory: URL
    internal let pins = MemoryHostKeyPins()
    internal let vault = MemoryPrivateKeys()
    internal let maker = FakeKeyMaker()
    internal let path = TestPathSource()
    internal let clock = TestClock()
    internal let clipboard = MemoryClipboard(nil)
    internal let core: FakeMobileCore
    internal let graph: AppGraph

    internal var connections: Connections {
        graph.connections
    }

    internal init() {
        directory = FileManager.default.temporaryDirectory.appending(path: "NgtermFeaturesTests-\(UUID().uuidString)")
        core = FakeMobileCore(hosts: pins)
        graph = AppGraph(
            machines: MachineStore(directory: directory),
            settings: SettingsStore(directory: directory),
            keys: AppKeys(store: KeyStore(directory: directory), vault: vault, maker: maker, comment: "ngterm@test"),
            pins: pins,
            stores: (
                attached: AttachedStore(directory: directory),
                clipboardTrust: ClipboardTrustStore(directory: directory)
            ),
            core: core,
            pathSource: path,
            clipboard: clipboard,
            clock: clock.read
        )
    }

    /// The tests' machine `m1`, build-01, signing in with `keyId`.
    internal static func machine(keyId: String) -> StoredMachine {
        machine(id: "m1", label: "build-01", host: "build-01.lan", keyId: keyId)
    }

    /// A machine for tests: `id`, named `label`, at `host`, signing in as nick with `keyId`.
    internal static func machine(id: String, label: String, host: String, keyId: String) -> StoredMachine {
        StoredMachine(id: id, label: label, host: host, port: StoredMachine.sshPort, user: "nick", keyId: keyId)
    }

    /// Makes the app's first key, as the first run would.
    internal func seedKey() async throws -> AppKey {
        try await graph.keys.generate()
    }

    /// Adds `machine` to the store.
    internal func seed(_ machine: StoredMachine) throws {
        try graph.machines.save(machine)
    }

    /// How many connects the core was asked for.
    internal func connects() -> Int {
        core.calls.count { $0.method == "connect" }
    }

    /// Waits until the connections have taken every event the core handed out.
    internal func settled() async {
        await Wait.settled(connections, core)
    }

    /// Deletes the test's directory.
    internal func remove() {
        try? FileManager.default.removeItem(at: directory)
    }

    deinit {
        // the directory goes with remove()
    }
}
