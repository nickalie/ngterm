import UIKit

/// A place in a terminal's text-input document, the composition (``TerminalTextInput``): an offset in UTF-16 units.
internal final class TerminalTextPosition: UITextPosition {
    /// The offset from the composition's start, in UTF-16 units.
    internal let offset: Int

    /// The place `offset` UTF-16 units into the composition.
    internal init(_ offset: Int) {
        self.offset = offset
    }

    deinit {
        // nothing to release
    }
}
