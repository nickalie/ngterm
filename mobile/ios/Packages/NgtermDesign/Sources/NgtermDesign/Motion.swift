import SwiftUI

/// How long motion takes (DESIGN.md > Motion): it answers an action, and nothing moves on its own.
///
/// With Reduce Motion on, every animation is a cut.
public enum Motion {
    /// A selection, a tint.
    public static let quick = 0.12
    /// A panel appearing.
    public static let standard = 0.22

    /// The animation for `duration`, or none with Reduce Motion on.
    public static func animation(_ duration: Double, reduceMotion: Bool) -> Animation? {
        reduceMotion ? nil : .easeOut(duration: duration)
    }
}
