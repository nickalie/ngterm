/// Makes `call` in a task of its own and hands back its answer or error, unless the caller's task is cancelled first.
///
/// A cancel ends the caller's wait at once with `CancellationError`, while `call` runs on to its end with its answer
/// dropped; a caller cancelled before the call starts never makes it. The facade's async calls go through this, as
/// Android's coroutines go through UniFFI's cancel: a core call posts its command to the core's actor, which carries it
/// out whoever waits for the answer.
package func abandoningOnCancel<Value: Sendable>(
    _ call: @escaping @Sendable () async throws -> Value
) async throws -> Value {
    let answer = CallAnswer<Value>()
    return try await withTaskCancellationHandler {
        try await withCheckedThrowingContinuation { continuation in
            guard answer.wait(continuation) else {
                return
            }
            Task.detached {
                do {
                    answer.resolve(.success(try await call()))
                } catch {
                    answer.resolve(.failure(error))
                }
            }
        }
    } onCancel: {
        answer.resolve(.failure(CancellationError()))
    }
}
