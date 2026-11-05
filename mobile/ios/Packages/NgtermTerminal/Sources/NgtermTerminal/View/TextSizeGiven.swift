import CoreFoundation

/// The text size a ``TerminalViewRepresentable`` last gave its view (its coordinator): the view keeps a size a pinch
/// gave it until the representable is given another (Settings changed it).
@preconcurrency
@MainActor
public final class TextSizeGiven {
    /// The size, in points.
    public internal(set) var textSize: CGFloat

    /// `textSize` given.
    public init(_ textSize: CGFloat) {
        self.textSize = textSize
    }

    deinit {
        // nothing to release
    }
}
