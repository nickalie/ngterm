import NgtermCoreBindings
import Synchronization

/// Stops an upload (``MobileCoreClient/uploadFile(machineId:name:bytes:progress:cancel:)``) while bytes are still to
/// go, so the machine keeps nothing of it.
///
/// A Swift task's cancel can't stop the core's work (its async calls run to their end), so the upload takes this:
/// the live core hands its token to the core, and the fake one waits on ``cancelled()``. Cancelling twice, or after
/// the upload ended, changes nothing.
public final class UploadCancel: Sendable {
    private struct State {
        var isCancelled = false
        var waiters: [CheckedContinuation<Void, Never>] = []
    }

    /// The core's own token, for the live core.
    package let token = NgtermCoreBindings.UploadCancel()
    private let state = Mutex(State())

    /// Whether ``cancel()`` has been called.
    public var isCancelled: Bool {
        state.withLock { $0.isCancelled }
    }

    /// A token nothing has been cancelled through yet.
    public init() {
        // nothing more: cancel() stops what it is given to
    }

    /// Stops the upload it was given to.
    public func cancel() {
        token.cancel()
        let waiters: [CheckedContinuation<Void, Never>] = state.withLock { state in
            guard !state.isCancelled else {
                return []
            }
            state.isCancelled = true
            defer { state.waiters = [] }
            return state.waiters
        }
        for waiter in waiters {
            waiter.resume()
        }
    }

    /// Waits until ``cancel()`` is called, at once when it was before.
    public func cancelled() async {
        await withCheckedContinuation { continuation in
            let wasCancelled = state.withLock { state in
                if !state.isCancelled {
                    state.waiters.append(continuation)
                }
                return state.isCancelled
            }
            if wasCancelled {
                continuation.resume()
            }
        }
    }

    deinit {
        // nothing to release: a token dropped uncancelled leaves its waiters waiting, as an upload never stopped does
    }
}
