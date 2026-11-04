import Synchronization

/// The answer of one ``abandoningOnCancel(_:)`` call: the first of the call's result and the cancel wins.
internal final class CallAnswer<Value: Sendable>: Sendable {
    private enum State {
        case pending
        case waiting(CheckedContinuation<Value, any Error>)
        case resolved(Result<Value, any Error>)
        case done
    }

    private let state = Mutex(State.pending)

    /// Hands the answer to `continuation` once there is one, at once when there is.
    ///
    /// - Returns: false when the answer is in already (a cancel came first), so the call is not to be made.
    internal func wait(_ continuation: CheckedContinuation<Value, any Error>) -> Bool {
        let early: Result<Value, any Error>? = state.withLock { state in
            guard case .resolved(let result) = state else {
                state = .waiting(continuation)
                return nil
            }
            state = .done
            return result
        }
        guard let early else {
            return true
        }
        continuation.resume(with: early)
        return false
    }

    /// The answer, unless one came before it.
    internal func resolve(_ result: Result<Value, any Error>) {
        let waiting: CheckedContinuation<Value, any Error>? = state.withLock { state in
            switch state {
            case .pending:
                state = .resolved(result)
                return nil

            case .waiting(let continuation):
                state = .done
                return continuation

            case .resolved, .done:
                return nil
            }
        }
        waiting?.resume(with: result)
    }

    deinit {
        // nothing to release: a waiting continuation is resumed before the answer goes
    }
}
