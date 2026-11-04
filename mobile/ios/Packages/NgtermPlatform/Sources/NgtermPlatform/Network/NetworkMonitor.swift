import NgtermCore
import Observation

/// Whether the device has a network, and a nudge to the core whenever a path comes up.
///
/// ``isOnline`` says whether there is a network. The nudge comes for a new path (its preferred interface changed,
/// Wi-Fi to cellular, say) or one regained after none; the core then retries at once what waits to reconnect.
///
/// Other updates (the same path turning expensive or constrained, an interface it doesn't prefer coming or going)
/// nudge nothing, and neither does the first path, which is where the monitor starts from.
@preconcurrency
@MainActor
@Observable
public final class NetworkMonitor {
    /// Whether the last path could carry traffic; true until the first path says otherwise.
    public private(set) var isOnline = true
    @ObservationIgnored private let source: any PathSource
    @ObservationIgnored private let onChange: @MainActor () -> Void
    @ObservationIgnored private var last: NetworkPath?

    /// A monitor that calls `onChange` for each new or regained path `source` reports.
    @preconcurrency
    public init(source: any PathSource, onChange: @escaping @MainActor () -> Void) {
        self.source = source
        self.onChange = onChange
    }

    /// A monitor that calls the core's ``MobileCoreClient/networkChanged()`` for each new or regained path
    /// `source` reports: the system's unless a test gives its own.
    public convenience init(core: any MobileCoreClient, source: (any PathSource)? = nil) {
        self.init(source: source ?? SystemPathSource()) {
            // the core only throws once it has stopped, when there is nothing left to retry
            try? core.networkChanged()
        }
    }

    /// Starts following the device's network, for the app's lifetime.
    public func start() {
        source.start { [weak self] path in
            self?.receive(path)
        }
    }

    /// Stops following it.
    public func stop() {
        source.stop()
    }

    private func receive(_ path: NetworkPath) {
        let previous = last
        last = path
        isOnline = path.isSatisfied
        guard let previous, path.isSatisfied else {
            return
        }
        if !previous.isSatisfied || previous.interfaces.first != path.interfaces.first {
            onChange()
        }
    }

    deinit {
        // nothing to release: the source is stopped through ``stop()``
    }
}
