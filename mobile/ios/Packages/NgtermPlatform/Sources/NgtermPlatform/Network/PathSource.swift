/// Where ``NetworkMonitor`` learns the device's network path from: the system's (``SystemPathSource``), or a test's.
@preconcurrency
@MainActor
public protocol PathSource: AnyObject {
    /// Starts telling `update` the current path, then each update of it, on the main actor.
    func start(_ update: @escaping @MainActor (NetworkPath) -> Void)

    /// Stops telling it.
    func stop()
}
