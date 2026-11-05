/// The short time the app keeps running once its last scene has left the foreground, before it lets go of its
/// connections: the system's (``SystemGrace``), or a test's, which it runs out by hand.
@preconcurrency
@MainActor
public protocol BackgroundGrace: AnyObject {
    /// Begins the grace; `expired` runs once when it runs out, unless ``end()`` comes first.
    func begin(expired: @escaping @MainActor () -> Void)

    /// Ends the grace before it runs out: its `expired` doesn't run.
    func end()
}
