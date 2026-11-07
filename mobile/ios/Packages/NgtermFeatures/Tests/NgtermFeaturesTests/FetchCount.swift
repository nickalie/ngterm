/// How many times a composer asked for its catalog.
@MainActor
internal final class FetchCount {
    internal var count = 0

    deinit {
        // nothing to release
    }
}
