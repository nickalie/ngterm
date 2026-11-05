import Foundation
import NgtermCore
import NgtermPlatform
import NgtermTerminal

/// What the app's screens work with, made once per process: the machines, the settings, the keys, the pinned host
/// keys, the connections over the mobile core, the attached sessions' live state and their shown panes, the programs'
/// clipboard reads, and whether the device is online.
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
    /// The attached sessions across machines, as the lists show them.
    public let attachedSessions: AttachedSessions
    /// The shown panes, and the pane each session has selected.
    public let panes: SessionPanes
    /// Keeps the screen awake while a session shows, when Settings asks for it.
    public let screenAwake = ScreenAwake()
    /// The device's clipboard, which programs write and read.
    public let clipboard: any TerminalClipboard
    /// The programs' clipboard reads waiting for the user.
    public let clipboardReads: ClipboardReads
    /// Whether the device is online; it nudges the core when a network comes up.
    public let network: NetworkMonitor
    private var isStarted = false

    /// The graph over its parts: `core` connects, `attached` keeps the attached set, `clipboardTrust` the machines
    /// trusted with the clipboard, and `pathSource` (the system's unless a test gives its own) tells the network; a
    /// reconnect counts down by `clock`, and programs write and read `clipboard` (the system's unless a test gives its
    /// own).
    @preconcurrency
    public init(
        machines: MachineStore,
        settings: SettingsStore,
        keys: AppKeys,
        pins: any HostKeyPins,
        stores: (attached: AttachedStore, clipboardTrust: ClipboardTrustStore),
        core: any MobileCoreClient,
        pathSource: (any PathSource)? = nil,
        clipboard: (any TerminalClipboard)? = nil,
        clock: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.machines = machines
        self.settings = settings
        self.keys = keys
        self.pins = pins
        let made = Connections(core: core, clock: clock)
        let board = clipboard ?? SystemClipboard()
        connections = made
        self.clipboard = board
        let streams = LiveSessions(connections: made, store: stores.attached, clipboard: board)
        live = streams
        attachedSessions = AttachedSessions(machines: machines, connections: made, live: streams)
        panes = SessionPanes(connections: made, live: streams)
        clipboardReads = ClipboardReads(connections: made, trust: stores.clipboardTrust, clipboard: board)
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

    /// The app has nothing to keep connected: every clipboard read still waiting is denied, then the core closes every
    /// connection quietly until ``Connections/resume()``.
    public func suspend() {
        clipboardReads.denyAll()
        connections.suspend()
    }

    deinit {
        // nothing to release: the process ends with it
    }
}
