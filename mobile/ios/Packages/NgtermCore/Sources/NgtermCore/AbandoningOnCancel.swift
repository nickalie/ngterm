/// Makes `call` in a task of its own and hands back its answer or error, unless the caller's task is cancelled first.
///
/// A cancel ends the caller's wait at once with `CancellationError`, while `call` runs on to its end with its answer
/// dropped; a caller cancelled before the call starts never makes it. The facade's async calls go through this, as
/// Android's coroutines go through UniFFI's cancel: a core call posts its command to the core's actor, which carries it
/// out whoever waits for the answer.
///
/// The call starts on the caller's own executor and runs there up to its first suspension before the caller waits, as
/// a Kotlin coroutine makes UniFFI's first poll: the core call's command is posted (api.rs posts on the first poll)
/// before anything the caller does next, so a synchronous call after a cancel (a disconnect, a hide) reaches the actor
/// after it.
nonisolated(nonsending) package func abandoningOnCancel<Value: Sendable>(
    _ call: @escaping @Sendable () async throws -> Value
) async throws -> Value {
    let answer = CallAnswer<Value>()
    return try await withTaskCancellationHandler {
        try await withCheckedThrowingContinuation { continuation in
            guard answer.wait(continuation) else {
                return
            }
            Task.immediate {
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
