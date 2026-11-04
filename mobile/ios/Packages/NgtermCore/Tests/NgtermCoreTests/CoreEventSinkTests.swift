import Dispatch
import Foundation
import NgtermCore
import Testing

/// The facade's `EventSink`: the core calls it on its actor thread, and it hands each event to the facade's stream.
@Suite("the core's event sink", .timeLimit(.minutes(1)))
internal struct CoreEventSinkTests {
    private static let posted = (0..<200).map { index in
        MobileEvent.paneDirty(pane: PaneRef(machineId: "m1", sessionId: "s1", paneId: "P\(index)"))
    }

    /// Posts `events` through `sink` from a thread of its own while this one, the main thread, waits for it: a
    /// callback that hopped onto the main actor synchronously would never return, and the wait would time out.
    @MainActor
    private static func postWhileHoldingTheMainActor(_ events: [MobileEvent], to sink: CoreEventSink) -> Bool {
        let done = DispatchSemaphore(value: 0)
        let poster = Thread {
            for event in events {
                sink.onEvent(event: event)
            }
            done.signal()
        }
        poster.start()
        return done.wait(timeout: .now() + .seconds(10)) == .success
    }

    @Test("events arrive on the stream in the order they were posted")
    internal func eventsArriveInOrder() async {
        let (events, sink) = CoreEventSink.makeStream()
        for event in Self.posted {
            sink.onEvent(event: event)
        }
        sink.finish()
        var received: [MobileEvent] = []
        for await event in events {
            received.append(event)
        }
        #expect(received == Self.posted)
    }

    @Test("the callback hands an event on without waiting on the main actor")
    @MainActor
    internal func callbackNeverWaitsOnTheMainActor() async {
        let (events, sink) = CoreEventSink.makeStream()
        let returned = Self.postWhileHoldingTheMainActor(Self.posted, to: sink)
        #expect(returned, "a callback waited on the main actor")
        sink.finish()
        var received: [MobileEvent] = []
        for await event in events {
            received.append(event)
        }
        #expect(received == Self.posted)
    }

    @Test("a finished stream takes no more events, and the callback still returns")
    internal func finishedStreamDropsLaterEvents() async {
        let (events, sink) = CoreEventSink.makeStream()
        sink.onEvent(event: Self.posted[0])
        sink.finish()
        sink.onEvent(event: Self.posted[1])
        var received: [MobileEvent] = []
        for await event in events {
            received.append(event)
        }
        #expect(received == [Self.posted[0]])
    }
}
