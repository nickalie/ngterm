import Network

/// The device's network path, from an `NWPathMonitor`: started once, stopped once.
@preconcurrency
@MainActor
public final class SystemPathSource: PathSource {
    private let monitor = NWPathMonitor()

    /// A source not started yet.
    public init() {
        // the monitor starts with ``start(_:)``
    }

    /// Starts the monitor on the main queue, telling `update` each path it reports.
    @preconcurrency
    public func start(_ update: @escaping @MainActor (NetworkPath) -> Void) {
        monitor.pathUpdateHandler = { path in
            let snapshot = NetworkPath(
                isSatisfied: path.status == .satisfied,
                interfaces: path.availableInterfaces.map(\.name)
            )
            MainActor.assumeIsolated {
                update(snapshot)
            }
        }
        monitor.start(queue: .main)
    }

    /// Cancels the monitor for good.
    public func stop() {
        monitor.cancel()
    }

    deinit {
        monitor.cancel()
    }
}
