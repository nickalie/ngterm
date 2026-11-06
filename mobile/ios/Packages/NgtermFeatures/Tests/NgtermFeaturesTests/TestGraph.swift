import Foundation
import NgtermCoreFakes
import NgtermPlatform

@testable import NgtermFeatures

/// The app's graph over fakes, as mobile/android's `TestGraph`: the core a ``FakeMobileCore`` pinning in memory, the
/// stores real ones in a directory of the test's own, the keys made by a ``FakeKeyMaker`` and kept in memory, the
/// network a ``TestPathSource``, the clock a ``TestClock``, the clipboard a ``MemoryClipboard``, the notifications a
/// ``RecordingNotices``, and the background refresh's core another ``FakeMobileCore`` (``refreshCore``), which keeps
/// the sessions of ``core``. ``remove()`` deletes the directory.
///
/// A graph made over another's `directory` reads the files that one wrote, as a process started afresh does.
@MainActor
internal final class TestGraph {
    internal let directory: URL
    internal let pins = MemoryHostKeyPins()
    internal let vault = MemoryPrivateKeys()
    internal let maker = FakeKeyMaker()
    internal let path = TestPathSource()
    internal let clock = TestClock()
    internal let clipboard = MemoryClipboard(nil)
    internal let notices = RecordingNotices()
    internal let core: FakeMobileCore
    internal let refreshCore: FakeMobileCore
    internal let graph: AppGraph

    internal var connections: Connections {
        graph.connections
    }

    /// The graph over a new directory of the test's own.
    internal convenience init() {
        let name = "NgtermFeaturesTests-\(UUID().uuidString)"
        self.init(directory: FileManager.default.temporaryDirectory.appending(path: name))
    }

    /// The graph over the stores in `directory`.
    internal init(directory: URL) {
        self.directory = directory
        core = FakeMobileCore(hosts: pins)
        let refreshing = FakeMobileCore(hosts: pins)
        refreshCore = refreshing
        let main = core
        graph = AppGraph(
            machines: MachineStore(directory: directory),
            settings: SettingsStore(directory: directory),
            keys: AppKeys(store: KeyStore(directory: directory), vault: vault, maker: maker, comment: "ngterm@test"),
            pins: pins,
            stores: GraphStores(directory: directory),
            core: core,
            notices: notices,
            pathSource: path,
            clipboard: clipboard,
            refreshCore: {
                // the machines have the same sessions, whichever core asks
                refreshing.sessions = main.sessions
                return refreshing
            },
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
