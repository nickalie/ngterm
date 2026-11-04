import NgtermPlatform

/// A network path source the test drives: ``send(_:)`` reports a path while the source is started.
@MainActor
internal final class FakePathSource: PathSource {
    private var update: (@MainActor (NetworkPath) -> Void)?

    /// Whether the source is started.
    internal var isStarted: Bool {
        update != nil
    }

    /// A source not started yet.
    internal init() {
        // the monitor starts it
    }

    internal func start(_ update: @escaping @MainActor (NetworkPath) -> Void) {
        self.update = update
    }

    internal func stop() {
        update = nil
    }

    /// Reports `path`, as the system would; nothing happens while the source is stopped.
    internal func send(_ path: NetworkPath) {
        update?(path)
    }

    deinit {
        // nothing to release
    }
}
