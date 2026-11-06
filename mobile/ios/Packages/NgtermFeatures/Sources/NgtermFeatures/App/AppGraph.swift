import Foundation
import NgtermCore
import NgtermPlatform
import NgtermTerminal

/// What the app's screens work with, made once per process: the machines, the settings, the keys, the pinned host
/// keys, the connections over the mobile core, the attached sessions' live state and their shown panes, the programs'
/// clipboard reads, the asks and the notifications, the status baseline and the background refresh, and whether the
/// device is online.
///
/// The app makes it over the linked core, the Keychain, Application Support and the system's notifications, or over
/// the fake core and memory for its UI tests; the tests make one over fakes. ``start()`` starts what runs on its own.
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
    /// The asks waiting for the user.
    public let asks: Asks
    /// What reaches the user as notifications.
    public let alerts: Alerts
    /// Where the app was asked to go by a notification's tap.
    public let openRequests = OpenRequests()
    /// The notification kinds turned off, and whether the app asked to notify.
    public let notificationPrefs: NotificationPrefsStore
    /// Where the notifications go, and whether the app may post them.
    public let notices: any NoticeCenter
    /// The last agent status seen of each attached session.
    public let baseline: StatusBaseline
    /// The background refresh's work.
    public let refresh: StatusRefresh
    /// Whether the device is online; it nudges the core when a network comes up.
    public let network: NetworkMonitor
    /// Whether ``start()`` has run: before it the core has connected nothing.
    public private(set) var isStarted = false

    /// The graph over its parts: `core` connects, `stores` keep the small state, and `notices` (the system's unless a
    /// test gives its own) takes the notifications; `pathSource` (the system's unless a test gives its own) tells the
    /// network; programs write and read `clipboard` (the system's unless a test gives its own); a background refresh
    /// runs on a core `refreshCore` makes (none: it refreshes nothing); and a reconnect counts down by `clock`.
    @preconcurrency
    public init(
        machines: MachineStore,
        settings: SettingsStore,
        keys: AppKeys,
        pins: any HostKeyPins,
        stores: GraphStores,
        core: any MobileCoreClient,
        notices: (any NoticeCenter)? = nil,
        pathSource: (any PathSource)? = nil,
        clipboard: (any TerminalClipboard)? = nil,
        refreshCore: (@MainActor () throws -> any MobileCoreClient)? = nil,
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
        let open = Asks(connections: made)
        asks = open
        notificationPrefs = stores.notificationPrefs
        let center = notices ?? SystemNotices()
        self.notices = center
        let posting = Alerts(
            connections: made,
            asks: open,
            machines: machines,
            prefs: stores.notificationPrefs,
            center: center,
            requests: openRequests
        )
        alerts = posting
        let seen = StatusBaseline(connections: made, store: stores.statusBaseline)
        baseline = seen
        refresh = StatusRefresh(
            machines: machines,
            attached: stores.attached,
            baseline: seen,
            alerts: posting,
            makeCore: refreshCore ?? { throw MobileError.Stopped }
        )
        network = NetworkMonitor(source: pathSource ?? SystemPathSource()) {
            made.networkChanged()
        }
    }

    /// Starts following the network and connects every machine the user added, once per process; later calls do
    /// nothing.
    ///
    /// A machines file the process couldn't read as it started (before the device's first unlock) is read again
    /// first.
    public func start() {
        guard !isStarted else {
            return
        }
        isStarted = true
        machines.readIfUnread()
        network.start()
        for machine in machines.machines {
            connections.connect(machine.core)
        }
    }

    /// The app has nothing to keep connected: every clipboard read still waiting is denied, the origins take their
    /// asks back (their notifications go), then the core closes every connection quietly until
    /// ``Connections/resume()``.
    public func suspend() {
        clipboardReads.denyAll()
        asks.takeBack()
        connections.suspend()
    }

    /// The app is back: asks open again, and the core connects again what ``suspend()`` closed (its reconnects
    /// reattach, and the origins hand over the asks still waiting).
    public func resume() {
        asks.resume()
        connections.resume()
    }

    deinit {
        // nothing to release: the process ends with it
    }
}
