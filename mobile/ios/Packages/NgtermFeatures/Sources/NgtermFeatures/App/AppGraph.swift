import Foundation
import NgtermCore
import NgtermPlatform

/// What the app's screens work with, made once per process: the machines, the settings, the keys, the pinned host
/// keys, the connections over the mobile core, the attached sessions' live state, and whether the device is online.
///
/// The app makes it over the linked core, the Keychain and Application Support, or over the fake core and memory for
/// its UI tests; the tests make one over fakes. ``start()`` starts what runs on its own.
@preconcurrency
@MainActor
public final class AppGraph {
    /// The machines the user added.
    public let machines: MachineStore
    /// The app's settings.
    public let settings: SettingsStore
    /// The app's ssh keys.
    public let keys: AppKeys
    /// The pinned host keys, which the core reads too.
    public let pins: any HostKeyPins
    /// The connections over the core.
    public let connections: Connections
    /// The attached sessions as the core tells them.
    public let live: LiveSessions
    /// Whether the device is online; it nudges the core when a network comes up.
    public let network: NetworkMonitor
    private var isStarted = false

    /// The graph over its parts: `core` connects, `attached` keeps the attached set, and `pathSource` (the system's
    /// unless a test gives its own) tells the network; a reconnect counts down by `clock`.
    @preconcurrency
    public init(
        machines: MachineStore,
        settings: SettingsStore,
        keys: AppKeys,
        pins: any HostKeyPins,
        attached: AttachedStore,
        core: any MobileCoreClient,
        pathSource: (any PathSource)? = nil,
        clock: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.machines = machines
        self.settings = settings
        self.keys = keys
        self.pins = pins
        let made = Connections(core: core, clock: clock)
        connections = made
        live = LiveSessions(connections: made, store: attached)
        network = NetworkMonitor(source: pathSource ?? SystemPathSource()) {
            made.networkChanged()
        }
    }

    /// Starts following the network and connects every machine the user added, once per process; later calls do
    /// nothing.
    public func start() {
        guard !isStarted else {
            return
        }
        isStarted = true
        network.start()
        for machine in machines.machines {
            connections.connect(machine.core)
        }
    }

    deinit {
        // nothing to release: the process ends with it
    }
}
