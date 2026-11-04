/// Where ``NetworkMonitor`` learns the device's network path from: the system's (``SystemPathSource``), or a test's.
@preconcurrency
@MainActor
public protocol PathSource: AnyObject {
    /// Starts telling `update` the current path, then each update of it, on the main actor. A source stopped can be
    /// started again, and reports again from the current path.
    func start(_ update: @escaping @MainActor (NetworkPath) -> Void)

    /// Stops telling it, until the next ``start(_:)``.
    func stop()
}
