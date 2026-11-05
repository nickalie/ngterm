import Foundation

/// How a fling slows down once the finger has left, as a list does: its speed falls by `rate` every millisecond.
public struct FlingDecay: Sendable {
    /// The slowest let-go, in points a second, that flings at all.
    public static let minimumSpeed: CGFloat = 50
    /// The speed under which a fling stops, in points a second.
    public static let stopSpeed: CGFloat = 10
    /// What a list's speed keeps of itself each millisecond: `UIScrollView.DecelerationRate.normal`.
    public static let normalRate = 0.998
    private static let millisecondsPerSecond = 1_000.0

    /// The speed it started at, in points a second.
    public let velocity: CGFloat
    private let rate: Double

    /// A fling let go at `velocity` points a second, slowing by `rate` per millisecond.
    public init(velocity: CGFloat, rate: Double = Self.normalRate) {
        self.velocity = velocity
        self.rate = rate
    }

    /// How far it has gone `seconds` after it was let go, in points.
    public func offset(after seconds: TimeInterval) -> CGFloat {
        let decay = log(rate) * Self.millisecondsPerSecond
        return velocity * CGFloat((pow(rate, seconds * Self.millisecondsPerSecond) - 1) / decay)
    }

    /// Its speed `seconds` after it was let go, in points a second.
    public func speed(after seconds: TimeInterval) -> CGFloat {
        velocity * CGFloat(pow(rate, seconds * Self.millisecondsPerSecond))
    }

    /// Whether it has stopped `seconds` after it was let go.
    public func isOver(after seconds: TimeInterval) -> Bool {
        abs(speed(after: seconds)) < Self.stopSpeed
    }
}
