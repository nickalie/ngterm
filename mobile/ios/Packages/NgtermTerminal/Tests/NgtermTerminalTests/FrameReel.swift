import Foundation
import Synchronization

/// Frames packed ahead, handed out in turn and round again: a pane whose output never stops, with no packing in the
/// time measured.
internal final class FrameReel: Sendable {
    private let frames: [Data]
    private let served = Mutex(0)

    /// How many frames were handed out.
    internal var count: Int {
        served.withLock { $0 }
    }

    /// A reel of `frames`, which holds at least one.
    internal init(_ frames: [Data]) {
        precondition(!frames.isEmpty, "a reel holds a frame")
        self.frames = frames
    }

    /// The next frame.
    internal func next() -> Data {
        let index = served.withLock { served in
            defer { served += 1 }
            return served
        }
        return frames[index % frames.count]
    }

    deinit {
        // nothing to release
    }
}
