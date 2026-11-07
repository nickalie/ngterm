#if NGTERM_FAKES
    import Synchronization

    /// Waits for the first of two waits to end; the other is left to end on its own (a fake's gate a test opens).
    internal final class FakeRace: Sendable {
        private let ended = Mutex(false)

        internal init() {
            // nothing more: first(_:_:) races
        }

        /// Returns once `one` or `other` has.
        internal func first(
            _ one: @escaping @Sendable () async -> Void,
            _ other: @escaping @Sendable () async -> Void
        ) async {
            await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                let finish: @Sendable () -> Void = { [self] in
                    let isFirst = ended.withLock { ended in
                        defer { ended = true }
                        return !ended
                    }
                    if isFirst {
                        continuation.resume()
                    }
                }
                Task {
                    await one()
                    finish()
                }
                Task {
                    await other()
                    finish()
                }
            }
        }

        deinit {
            // nothing to release
        }
    }
#endif
