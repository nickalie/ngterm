#if NGTERM_FAKES
    import Synchronization

    /// A gate a fake's call waits at until it opens, with the value it opens with.
    ///
    /// A test holds a call with it (``FakeMobileCore/hold(_:)``), and the fake waits at one for the user's answer (a
    /// host key, a sign-in). It opens once: every call waiting goes through, as does every later one.
    public final class FakeGate<Value: Sendable>: Sendable {
        private struct State {
            var value: Value?
            var waiters: [CheckedContinuation<Value, Never>] = []
            var reached = false
            var arrivals: [CheckedContinuation<Void, Never>] = []
        }

        private let state = Mutex(State())

        /// Whether it has opened.
        public var isOpen: Bool {
            state.withLock { $0.value != nil }
        }

        /// A closed gate.
        public init() {
            // nothing more: it opens with open(with:)
        }

        /// Opens it with `value`; a gate already open stays as it opened.
        public func open(with value: Value) {
            let waiters: [CheckedContinuation<Value, Never>] = state.withLock { state in
                guard state.value == nil else {
                    return []
                }
                state.value = value
                defer { state.waiters = [] }
                return state.waiters
            }
            for waiter in waiters {
                waiter.resume(returning: value)
            }
        }

        /// Waits until it opens, and hands back the value it opened with.
        public func wait() async -> Value {
            await withCheckedContinuation { continuation in
                let (opened, arrivals): (Value?, [CheckedContinuation<Void, Never>]) = state.withLock { state in
                    state.reached = true
                    defer { state.arrivals = [] }
                    if state.value == nil {
                        state.waiters.append(continuation)
                    }
                    return (state.value, state.arrivals)
                }
                for arrival in arrivals {
                    arrival.resume()
                }
                if let opened {
                    continuation.resume(returning: opened)
                }
            }
        }

        /// Waits until a call has come to the gate: one is waiting at it, or went through.
        public func reached() async {
            await withCheckedContinuation { continuation in
                let reached = state.withLock { state in
                    if !state.reached {
                        state.arrivals.append(continuation)
                    }
                    return state.reached
                }
                if reached {
                    continuation.resume()
                }
            }
        }

        deinit {
            // nothing to release: a gate dropped closed leaves its waiters waiting, as a call never answered does
        }
    }

    extension FakeGate where Value == Void {
        /// Opens it.
        public func open() {
            open(with: ())
        }
    }
#endif
