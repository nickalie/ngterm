import UIKit

/// A terminal's text-input document (mobile/android's `TerminalInputConnection`): only the composition an input method
/// has in progress (a CJK input method's, a dead key's, dictation's), which waits at the cursor until it commits.
///
/// What commits goes to the program at once, so nothing stays in the document after: a deletion past the composition is
/// the program's backspace. Offsets and ranges are in UTF-16 units, as UIKit's are.
@MainActor
internal final class TerminalTextInput {
    /// The composition in progress; empty while nothing is composed.
    internal private(set) var composed = ""
    /// The selection within it: the caret the input method keeps.
    internal private(set) var selection = NSRange(location: 0, length: 0)
    /// The system's text input, told when the document changes from outside it.
    internal weak var delegate: (any UITextInputDelegate)?
    /// The style the system asked the composition be drawn in (kept, unused: it draws as the terminal does).
    internal var markedStyle: [NSAttributedString.Key: Any] = [:]
    /// Where it draws.
    internal let layer = CompositionLayer()

    /// Its length, in UTF-16 units.
    internal var length: Int {
        composed.utf16.count
    }

    /// Whether a composition is in progress.
    internal var isComposing: Bool {
        !composed.isEmpty
    }

    internal init() {
        layer.isHidden = true
    }

    /// The input method composes `text`, its caret or selection at `selected`.
    internal func compose(_ text: String, selected: NSRange) {
        composed = text
        selection = clamped(selected)
    }

    /// The input method moves its caret or selection to `range`.
    internal func select(_ range: NSRange) {
        selection = clamped(range)
    }

    /// Ends the composition, giving what it held.
    internal func take() -> String {
        let text = composed
        composed = ""
        selection = NSRange(location: 0, length: 0)
        return text
    }

    /// The composition's text in `range`; `nil` for a range past it.
    internal func text(in range: NSRange) -> String? {
        guard range.location >= 0, NSMaxRange(range) <= length, let span = Range(range, in: composed) else {
            return nil
        }
        return String(composed[span])
    }

    /// Puts `text` in place of `range` of the composition, the caret after it.
    internal func replace(_ range: NSRange, with text: String) {
        let replaced = clamped(range)
        guard let span = Range(replaced, in: composed) else {
            return
        }
        composed.replaceSubrange(span, with: text)
        selection = NSRange(location: replaced.location + text.utf16.count, length: 0)
    }

    /// Deletes the selection, or the character before the caret, from the composition; false when nothing is composed
    /// before the caret, so the deletion is the program's.
    internal func deleteBackward() -> Bool {
        if selection.length > 0 {
            replace(selection, with: "")
            return true
        }
        guard selection.location > 0 else {
            return false
        }
        // the whole character before the caret, a surrogate pair or a cluster with it
        let caret = String.Index(utf16Offset: selection.location, in: composed)
        let start = composed.index(before: caret)
        replace(NSRange(start..<caret, in: composed), with: "")
        return true
    }

    // `range` kept inside the composition
    private func clamped(_ range: NSRange) -> NSRange {
        let start = min(max(range.location, 0), length)
        let end = min(max(start + max(range.length, 0), start), length)
        return NSRange(location: start, length: end - start)
    }

    deinit {
        // nothing to release
    }
}
