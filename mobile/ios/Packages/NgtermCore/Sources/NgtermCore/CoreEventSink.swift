import NgtermCoreBindings

/// The core's `EventSink`, which feeds the facade's event stream.
///
/// The core calls it on its actor thread, and it yields each event into the stream and returns. It never waits, on
/// the main actor or on the stream's consumer: the stream buffers without bound, since events are small and losing one
/// is worse.
package final class CoreEventSink: NgtermCoreBindings.EventSink {
    private let continuation: AsyncStream<MobileEvent>.Continuation

    private init(_ continuation: AsyncStream<MobileEvent>.Continuation) {
        self.continuation = continuation
    }

    /// A stream and the sink that feeds it.
    package static func makeStream() -> (events: AsyncStream<MobileEvent>, sink: CoreEventSink) {
        let (stream, feed) = AsyncStream.makeStream(of: MobileEvent.self, bufferingPolicy: .unbounded)
        return (stream, Self(feed))
    }

    /// Hands `event` to the stream, after the ones before it; a finished stream drops it.
    package func onEvent(event: MobileEvent) {
        continuation.yield(event)
    }

    /// Ends the stream: its consumer gets what it holds, then the end.
    package func finish() {
        continuation.finish()
    }

    deinit {
        // the stream ends with its facade (finish()), not with the sink, which the core may outlive
    }
}
