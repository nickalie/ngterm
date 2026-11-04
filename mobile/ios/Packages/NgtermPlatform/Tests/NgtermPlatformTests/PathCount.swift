/// How many paths a source reported.
@MainActor
internal final class PathCount {
    /// How many so far.
    internal var reported = 0

    /// None yet.
    internal init() {
        // the source's updates count
    }

    deinit {
        // nothing to release
    }
}
