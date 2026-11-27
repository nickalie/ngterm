import Foundation
import NgtermCore
import NgtermPlatform
import NgtermTerminal
import Observation

/// What the app's screens work with, made once per process: the machines, the settings, the keys, the pinned host
/// keys, the connections over the mobile core, the session create under way, the attached sessions' live state, their
/// shown panes and the agents' chats, the programs' clipboard reads, the asks and the notifications, the status
/// baseline and the background refresh, and whether the device is online.
///
/// The app makes it over the linked core, the Keychain, Application Support and the system's notifications, or over
/// the fake core and memory for its UI tests; the tests make one over fakes. ``start()`` starts what runs on its own.
/// Making it removes the exported key files an earlier run left staged.
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
    /// The session create under way on this device, whichever screen began it.
    public let creates = Creates()
    /// The shown panes, and the pane each session has selected.
    public let panes: SessionPanes
    /// Which panes running an agent show as a chat, and their chats.
    public let chats: PaneChats
    /// Keeps the screen awake while a session shows, when Settings asks for it.
    public let screenAwake = ScreenAwake()
    /// What exporting a key needs of the platform.
    public let exports: KeyExportServices
    /// The device's clipboard, which programs write and read.
    public let clipboard: any TerminalClipboard
    /// The programs' clipboard reads waiting for the user.
    public let clipboardReads: ClipboardReads
    /// The asks waiting for the user.
    public let asks: Asks
    /// The picks the attached sessions' origins handed over.
    public let picks: Picks
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
    /// Where the app writes its own lines of the diagnostic log.
    public let diagnostics: Diagnostics
    /// Where the Logs page writes the files it shares.
    public let logExports: URL
    /// Whether ``start()`` has run: before it the core has connected nothing.
    public private(set) var isStarted = false
    // the trusted machines and the sessions' own chat views, read again with the other stores
    private let clipboardTrust: ClipboardTrustStore
    private let sessionChatViews: SessionChatViewsStore
    private var followingTimeout: Task<Void, Never>?

    /// The graph over its parts: `core` connects, `stores` keep the small state, and `notices` (the system's unless a
    /// test gives its own) takes the notifications; `pathSource` (the system's unless a test gives its own) tells the
    /// network; programs write and read `clipboard` (the system's unless a test gives its own); a background refresh
    /// runs on a core `refreshCore` makes (none: it refreshes nothing); a key is exported through `exports` (the
    /// system's unless a test gives its own); a reconnect counts down by `clock`; and the app's own lines go to the
    /// diagnostic `log` (none: nowhere), whose shared files are written under `logExports`.
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
        exports: KeyExportServices? = nil,
        clock: @escaping @Sendable () -> Date = { Date() },
        log: (any AppLogProtocol)? = nil,
        logExports: URL = FileManager.default.temporaryDirectory.appending(path: "logs-export")
    ) {
        let lines = Diagnostics(log: log)
        diagnostics = lines
        self.logExports = logExports
        self.machines = machines
        self.settings = settings
        self.keys = keys
        self.pins = pins
        self.exports = exports ?? KeyExportServices()
        // a key an earlier run staged for the file screen stays there if that run ended before the screen did
        self.exports.files.discardAll()
        let made = Connections(core: core, clock: clock)
        let board = clipboard ?? SystemClipboard()
        connections = made
        self.clipboard = board
        let streams = LiveSessions(connections: made, store: stores.attached, clipboard: board)
        live = streams
        attachedSessions = AttachedSessions(machines: machines, connections: made, live: streams)
        let shown = SessionPanes(connections: made, live: streams)
        panes = shown
        sessionChatViews = stores.sessionChatViews
        chats = PaneChats(
            connections: made,
            live: streams,
            panes: shown,
            settings: settings,
            store: stores.sessionChatViews,
            diagnostics: lines
        )
        clipboardTrust = stores.clipboardTrust
        let keyClipboard = self.exports.clipboard
        clipboardReads = ClipboardReads(connections: made, trust: stores.clipboardTrust, clipboard: board) {
            keyClipboard.mayHoldKey
        }
        let open = Asks(connections: made)
        asks = open
        picks = Picks(connections: made)
        notificationPrefs = stores.notificationPrefs
        let center = notices ?? SystemNotices()
        self.notices = center
        let posting = Alerts(
            connections: made,
            asks: open,
            live: streams,
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
            alerts: posting
        ) {
            guard let refreshCore else {
                throw MobileError.Stopped
            }
            let refreshing = try refreshCore()
            refreshing.setConnectTimeoutSecs(UInt32(clamping: settings.settings.connectTimeout))
            return refreshing
        }
        network = NetworkMonitor(source: pathSource ?? SystemPathSource()) {
            made.networkChanged()
        } onOnline: { isOnline in
            lines.info(.app, isOnline ? "network online" : "network offline")
        }
        followConnectTimeout()
    }

    /// Reads again every file of the user's that the process couldn't read as it started.
    ///
    /// The system starts the process for a background refresh before the device's first unlock, and its window may
    /// open later. The machines, the keys, the settings, the notification choices and the trusted machines show what
    /// their files hold once they can be read; a store that has read its file keeps what it has.
    ///
    /// A window calls it before it decides on the first run (which shows while there is no key), and ``start()`` before
    /// it connects.
    public func readUnreadStores() {
        machines.readIfUnread()
        keys.readIfUnread()
        settings.readIfUnread()
        notificationPrefs.readIfUnread()
        clipboardTrust.readIfUnread()
        sessionChatViews.readIfUnread()
    }

    /// Starts following the network and connects every machine the user added, once per process; later calls do
    /// nothing.
    ///
    /// The files the process couldn't read as it started are read again first (``readUnreadStores()``).
    public func start() {
        guard !isStarted else {
            return
        }
        isStarted = true
        readUnreadStores()
        network.start()
        for machine in machines.machines {
            connections.connect(machine.core)
        }
    }

    /// The app has nothing to keep connected: every clipboard read still waiting is denied, the origins take their
    /// asks and picks back (their notifications go), then the core closes every connection quietly until
    /// ``Connections/resume()``.
    public func suspend() {
        clipboardReads.denyAll()
        asks.takeBack()
        picks.takeBack()
        connections.suspend()
    }

    /// The app is back: asks and picks open again, and the core connects again what ``suspend()`` closed (its
    /// reconnects reattach, and the origins hand over the asks and picks still waiting).
    public func resume() {
        asks.resume()
        picks.resume()
        connections.resume()
    }

    // hands the core Settings' connect timeout now and after each change of the settings
    private func followConnectTimeout() {
        connections.setConnectTimeout(seconds: settings.settings.connectTimeout)
        followingTimeout = Task { [weak self] in
            let seconds = Observations { @MainActor [weak self] in self?.settings.settings.connectTimeout }
            for await now in seconds {
                guard let self, let now else {
                    return
                }
                connections.setConnectTimeout(seconds: now)
            }
        }
    }

    deinit {
        // nothing to release: the process ends with it
    }
}
