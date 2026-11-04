import NgtermCore
import Testing

/// How the facade's async calls meet a cancelled task: the caller stops waiting, the call it made runs on.
@Suite("a call abandoned on cancel", .timeLimit(.minutes(1)))
internal struct AbandoningOnCancelTests {
    private struct Refused: Error, Equatable {}

    @Test("the call's answer, or its error, is the caller's")
    internal func answerAndErrorPassThrough() async throws {
        #expect(try await abandoningOnCancel { "answer" } == "answer")
        await #expect(throws: Refused()) {
            try await abandoningOnCancel { () throws -> Int in throw Refused() }
        }
    }

    @Test("a cancel ends the caller's wait at once, while the call runs on to its end")
    internal func cancelEndsTheWaitOnly() async throws {
        let (started, start) = AsyncStream.makeStream(of: Void.self)
        let (opened, open) = AsyncStream.makeStream(of: Void.self)
        let (ended, end) = AsyncStream.makeStream(of: Void.self)
        let caller = Task {
            try await abandoningOnCancel {
                start.yield()
                for await _ in opened {
                    break
                }
                end.yield()
                return "late"
            }
        }
        for await _ in started {
            break
        }
        caller.cancel()
        await #expect(throws: CancellationError.self) {
            try await caller.value
        }
        open.yield()
        var ends = ended.makeAsyncIterator()
        #expect(await ends.next() != nil, "the call ended")
    }

    @Test("the call starts on the caller's executor, before the caller waits")
    @MainActor
    internal func callStartsBeforeTheCallerWaits() async {
        let started = Counter()
        let (opened, open) = AsyncStream.makeStream(of: Void.self)
        // runs on the main actor up to its first suspension: the caller's wait for the call's answer
        let caller = Task.immediate { @MainActor in
            try await abandoningOnCancel {
                started.add()
                for await _ in opened {
                    break
                }
            }
        }
        #expect(started.count == 1, "the call started before the caller waited")
        caller.cancel()
        await #expect(throws: CancellationError.self) {
            try await caller.value
        }
        open.finish()
    }

    @Test("a task cancelled before the call never makes it")
    internal func cancelledTaskMakesNoCall() async {
        let made = Counter()
        let caller = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            try await abandoningOnCancel { made.add() }
        }
        await #expect(throws: CancellationError.self) {
            try await caller.value
        }
        #expect(made.count == .zero)
    }
}
