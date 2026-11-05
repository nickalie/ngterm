import UIKit

/// A span of a terminal's text-input document, the composition (``TerminalTextInput``), in UTF-16 units.
internal final class TerminalTextRange: UITextRange {
    /// The span.
    internal let range: NSRange

    /// Where it starts.
    override internal var start: UITextPosition {
        TerminalTextPosition(range.location)
    }

    /// Where it ends.
    override internal var end: UITextPosition {
        TerminalTextPosition(NSMaxRange(range))
    }

    /// Whether it spans nothing.
    override internal var isEmpty: Bool {
        range.length == 0
    }

    /// The span `range`.
    internal init(_ range: NSRange) {
        self.range = range
    }

    /// The span from `start` to `end`, in either order.
    internal convenience init(from start: Int, to end: Int) {
        self.init(NSRange(location: min(start, end), length: abs(end - start)))
    }

    deinit {
        // nothing to release
    }
}
