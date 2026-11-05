import NgtermPlatform

/// The app's ``Lifecycle`` over its graph, and its background refresh's place in it.
///
/// What the app holds is its attached sessions, and a machine waiting for the user to sign in. Letting go is the
/// graph's ``AppGraph/suspend()`` (clipboard reads denied, asks taken back with their notifications, then the core's
/// suspend); coming back is the core's resume, whose reconnects reattach. A refresh is asked for while sessions are
/// attached, and runs only while the core is suspended or not yet started: in the foreground or the grace, the streams
/// tell the statuses themselves.
@preconcurrency
@MainActor
public final class AppLifecycle {
    /// The app's life across its scenes.
    public let lifecycle: Lifecycle
    private let graph: AppGraph
    private let scheduler: any RefreshScheduling

    /// The life of the app over `graph`, keeping `grace` in the background and asking `scheduler` for refreshes.
    public init(graph: AppGraph, grace: any BackgroundGrace, scheduler: any RefreshScheduling) {
        self.graph = graph
        self.scheduler = scheduler
        let connections = graph.connections
        lifecycle = Lifecycle(grace: grace) {
            if connections.isSigningIn {
                return .signIn
            }
            return connections.attached.values.contains { !$0.isEmpty } ? .sessions : .nothing
        } suspend: {
            graph.suspend()
        } resume: {
            connections.resume()
        }
        connections.watchAttached { [weak self] attached in
            self?.attachedChanged(attached)
        }
        connections.listen { [weak self] event in
            // a sign-in asked for, or ended (the machine's state changed), changes what the app holds
            switch event {
            case .signInRequested, .machineState:
                self?.lifecycle.holdsChanged()

            default:
                break
            }
        }
    }

    /// The background refresh's work: once while the core is suspended or not yet started, then asked for again while
    /// a machine has sessions attached.
    public func refresh() async {
        if lifecycle.isSuspended || !graph.isStarted {
            await graph.refresh.run()
        }
        if graph.refresh.hasWork {
            scheduler.schedule()
        }
    }

    private func attachedChanged(_ attached: [String: Set<String>]) {
        lifecycle.holdsChanged()
        if attached.values.contains(where: { !$0.isEmpty }) {
            scheduler.schedule()
        } else {
            scheduler.cancel()
        }
    }

    deinit {
        // nothing to release: the connections hold only weak references back
    }
}
