import NgtermCoreFakes
import NgtermPlatform
import Testing

/// The network monitor: the core is nudged on a new or regained path, not on every update.
@MainActor
@Suite("the network monitor")
internal struct NetworkMonitorTests {
    private static let wifi = NetworkPath(isSatisfied: true, interfaces: ["en0"], gateways: ["192.168.1.1:0"])
    private static let wifiAndCellular = NetworkPath(
        isSatisfied: true, interfaces: ["en0", "pdp_ip0"], gateways: ["192.168.1.1:0"]
    )
    private static let otherWifi = NetworkPath(isSatisfied: true, interfaces: ["en0"], gateways: ["10.0.0.1:0"])
    private static let cellular = NetworkPath(isSatisfied: true, interfaces: ["pdp_ip0"], gateways: ["10.64.0.1:0"])
    private static let offline = NetworkPath(isSatisfied: false, interfaces: [])

    private let core = FakeMobileCore()
    private let source = FakePathSource()

    private var nudges: Int {
        core.calls.count { $0.method == "networkChanged" }
    }

    // the source holds the monitor weakly, so each test keeps the one it starts
    private func startedMonitor() -> NetworkMonitor {
        let monitor = NetworkMonitor(core: core, source: source)
        monitor.start()
        return monitor
    }

    @Test("the first path is where the monitor starts from: no nudge")
    internal func firstPathIsTheBaseline() {
        let monitor = startedMonitor()
        #expect(source.isStarted)
        source.send(Self.wifi)
        #expect(nudges == 0)
        #expect(monitor.isOnline)
    }

    @Test("a path regained after none nudges the core once, and going offline nudges nothing")
    internal func regainedPath() {
        let monitor = startedMonitor()
        source.send(Self.wifi)
        source.send(Self.offline)
        #expect(!monitor.isOnline)
        #expect(nudges == 0)
        source.send(Self.offline)
        source.send(Self.wifi)
        #expect(monitor.isOnline)
        #expect(nudges == 1)
    }

    @Test("starting offline, the first path that comes up nudges the core")
    internal func startsOffline() {
        let monitor = startedMonitor()
        source.send(Self.offline)
        #expect(!monitor.isOnline)
        source.send(Self.cellular)
        #expect(nudges == 1)
    }

    @Test("a new preferred interface nudges the core: Wi-Fi to cellular and back")
    internal func newPath() {
        let monitor = startedMonitor()
        source.send(Self.wifi)
        source.send(Self.cellular)
        #expect(nudges == 1)
        source.send(Self.wifiAndCellular)
        #expect(nudges == 2)
        #expect(monitor.isOnline)
    }

    @Test("another network behind the same interface nudges the core: one Wi-Fi straight to another")
    internal func otherNetworkSameInterface() {
        let monitor = startedMonitor()
        source.send(Self.wifi)
        source.send(Self.otherWifi)
        #expect(nudges == 1)
        source.send(Self.otherWifi)
        #expect(nudges == 1)
        #expect(monitor.isOnline)
    }

    @Test("updates of the same path nudge nothing: repeats, and an interface it doesn't prefer coming or going")
    internal func sameTimesMany() {
        let monitor = startedMonitor()
        source.send(Self.wifi)
        for path in [Self.wifi, Self.wifiAndCellular, Self.wifi, Self.wifiAndCellular, Self.wifiAndCellular] {
            source.send(path)
        }
        #expect(nudges == 0)
        #expect(monitor.isOnline)
    }

    @Test("a stopped monitor stops its source")
    internal func stop() {
        let monitor = startedMonitor()
        source.send(Self.offline)
        monitor.stop()
        #expect(!source.isStarted)
        source.send(Self.wifi)
        #expect(nudges == 0)
        #expect(!monitor.isOnline)
    }

    @Test("a monitor started again follows the network again, from the last path it had")
    internal func restart() {
        let monitor = startedMonitor()
        source.send(Self.wifi)
        monitor.stop()
        monitor.start()
        #expect(source.isStarted)
        source.send(Self.wifi)
        #expect(nudges == 0)
        source.send(Self.cellular)
        #expect(nudges == 1)
        monitor.stop()
        monitor.start()
        source.send(Self.offline)
        #expect(!monitor.isOnline)
    }

    @Test("the monitor calls what it was given for a new path")
    internal func closure() {
        var changes = 0
        let monitor = NetworkMonitor(source: source) {
            changes += 1
        }
        monitor.start()
        source.send(Self.offline)
        source.send(Self.wifi)
        #expect(changes == 1)
    }
}
