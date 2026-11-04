import Network

/// The device's network path, from an `NWPathMonitor`: a new one each time the source starts, since one that was
/// cancelled reports nothing again.
@preconcurrency
@MainActor
public final class SystemPathSource: PathSource {
    private var monitor: NWPathMonitor?

    /// A source not started yet.
    public init() {
        // a monitor is made by ``start(_:)``
    }

    /// Starts a new monitor on the main queue, telling `update` each path it reports; one already started stops.
    @preconcurrency
    public func start(_ update: @escaping @MainActor (NetworkPath) -> Void) {
        monitor?.cancel()
        let next = NWPathMonitor()
        next.pathUpdateHandler = { path in
            let snapshot = NetworkPath(
                isSatisfied: path.status == .satisfied,
                interfaces: path.availableInterfaces.map(\.name),
                gateways: Set(path.gateways.map { String(describing: $0) })
            )
            MainActor.assumeIsolated {
                update(snapshot)
            }
        }
        next.start(queue: .main)
        monitor = next
    }

    /// Cancels the monitor; ``start(_:)`` makes a new one.
    public func stop() {
        monitor?.cancel()
        monitor = nil
    }

    deinit {
        monitor?.cancel()
    }
}
