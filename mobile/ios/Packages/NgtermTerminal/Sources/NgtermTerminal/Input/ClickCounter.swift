import CoreGraphics
import Foundation

/// Counts clicks (or taps) in a row: a click within ``interval`` of the last, and within `reach` of it, counts on
/// from it, as a double and a triple click do.
internal struct ClickCounter {
    /// How long after a click the next still counts on from it, in seconds (UIKit's double tap).
    internal static let interval: TimeInterval = 0.35

    private var last: (point: CGPoint, time: TimeInterval, count: Int)?

    /// A click at `point` at `time` (seconds): how many clicks in a row it makes, counting on from the last one when
    /// it came soon enough and within `reach` points of it.
    internal mutating func click(at point: CGPoint, time: TimeInterval, reach: CGFloat) -> Int {
        var count = 1
        if let last, time - last.time <= Self.interval, hypot(point.x - last.point.x, point.y - last.point.y) <= reach {
            count = last.count + 1
        }
        last = (point, time, count)
        return count
    }
}
