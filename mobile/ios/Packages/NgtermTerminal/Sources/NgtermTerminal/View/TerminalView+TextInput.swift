import UIKit

/// The soft keyboard's text input (mobile/android's `TerminalInputConnection`): what it commits goes to the program as
/// typed text (line breaks as Enter), what it composes waits at the cursor until it commits, and its delete key with
/// nothing composed is the program's backspace. The document is only the composition (``TerminalTextInput``).
extension TerminalView: UITextInput {
    /// Always: the program may have text before the cursor, so the keyboard's delete key always sends.
    public var hasText: Bool {
        true
    }

    /// The composition's span; `nil` while nothing is composed.
    public var markedTextRange: UITextRange? {
        textInput.isComposing ? TerminalTextRange(NSRange(location: 0, length: textInput.length)) : nil
    }

    // swiftlint:disable discouraged_optional_collection - UITextInput declares it optional
    /// The style the system asks the composition be drawn in.
    public var markedTextStyle: [NSAttributedString.Key: Any]? {
        get { textInput.markedStyle }
        set { textInput.markedStyle = newValue ?? [:] }
    }
    // swiftlint:enable discouraged_optional_collection

    /// The input method's caret or selection, within the composition.
    public var selectedTextRange: UITextRange? {
        get { TerminalTextRange(textInput.selection) }
        set {
            if let range = newValue as? TerminalTextRange {
                textInput.select(range.range)
            }
        }
    }

    /// The composition's start.
    public var beginningOfDocument: UITextPosition {
        TerminalTextPosition(0)
    }

    /// The composition's end.
    public var endOfDocument: UITextPosition {
        TerminalTextPosition(textInput.length)
    }

    /// The system's text input, told when the composition changes from outside it.
    public var inputDelegate: (any UITextInputDelegate)? {
        get { textInput.delegate }
        set { textInput.delegate = newValue }
    }

    /// Characters, words and lines of the composition, as a string's.
    public var tokenizer: any UITextInputTokenizer {
        UITextInputStringTokenizer(textInput: self)
    }

    /// The view the composition's rects are in: this one.
    public var textInputView: UIView {
        self
    }

    /// The soft keyboard (or the text system, for a hardware key) committed `text`: what it composed is done with, and
    /// `text` goes to the program, with the sticky modifiers on its first character.
    public func insertText(_ text: String) {
        if textInput.isComposing {
            _ = textInput.take()
            placeComposition()
        }
        keyboard.type(text, to: self)
    }

    /// The delete key: it takes from the composition, or with nothing composed before the caret it is the program's
    /// backspace.
    public func deleteBackward() {
        if textInput.deleteBackward() {
            placeComposition()
        } else {
            keyboard.press("backspace", to: self)
        }
    }

    /// Dictation's text, as typed.
    public func insertDictationResult(_ dictationResult: [UIDictationPhrase]) {
        insertText(dictationResult.map(\.text).joined())
    }

    /// The input method composes `markedText`, its caret at `selectedRange`: it shows at the cursor.
    public func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
        textInput.compose(markedText ?? "", selected: selectedRange)
        placeComposition()
    }

    /// The input method commits its composition as it stands: it goes to the program.
    public func unmarkText() {
        let text = textInput.take()
        placeComposition()
        if !text.isEmpty {
            keyboard.type(text, to: self)
        }
    }

    /// The composition's text in `range`.
    public func text(in range: UITextRange) -> String? {
        (range as? TerminalTextRange).flatMap { textInput.text(in: $0.range) }
    }

    /// Puts `text` in place of `range`: within the composition while there is one, else typed.
    public func replace(_ range: UITextRange, withText text: String) {
        guard textInput.isComposing, let span = range as? TerminalTextRange else {
            insertText(text)
            return
        }
        textInput.replace(span.range, with: text)
        placeComposition()
    }

    /// The span from `fromPosition` to `toPosition`.
    public func textRange(from fromPosition: UITextPosition, to toPosition: UITextPosition) -> UITextRange? {
        guard let start = fromPosition as? TerminalTextPosition, let end = toPosition as? TerminalTextPosition else {
            return nil
        }
        return TerminalTextRange(from: start.offset, to: end.offset)
    }

    /// The place `offset` units from `position`; `nil` past the composition's ends.
    public func position(from position: UITextPosition, offset: Int) -> UITextPosition? {
        guard let from = position as? TerminalTextPosition else {
            return nil
        }
        let moved = from.offset + offset
        return (0...textInput.length).contains(moved) ? TerminalTextPosition(moved) : nil
    }

    /// The place `offset` units from `position` toward `direction`: the composition is one line, so up and down go to
    /// its ends.
    public func position(
        from position: UITextPosition,
        in direction: UITextLayoutDirection,
        offset: Int
    ) -> UITextPosition? {
        switch direction {
        case .left:
            self.position(from: position, offset: -offset)

        case .right:
            self.position(from: position, offset: offset)

        case .up:
            beginningOfDocument

        case .down:
            endOfDocument

        @unknown default:
            nil
        }
    }

    /// How `position` stands to `other`.
    public func compare(_ position: UITextPosition, to other: UITextPosition) -> ComparisonResult {
        let first = (position as? TerminalTextPosition)?.offset ?? 0
        let second = (other as? TerminalTextPosition)?.offset ?? 0
        if first == second {
            return .orderedSame
        }
        return first < second ? .orderedAscending : .orderedDescending
    }

    /// The units from `from` to `toPosition`.
    public func offset(from: UITextPosition, to toPosition: UITextPosition) -> Int {
        ((toPosition as? TerminalTextPosition)?.offset ?? 0) - ((from as? TerminalTextPosition)?.offset ?? 0)
    }

    /// The end of `range` farthest toward `direction`.
    public func position(within range: UITextRange, farthestIn direction: UITextLayoutDirection) -> UITextPosition? {
        direction == .left || direction == .up ? range.start : range.end
    }

    /// The span from `position` to the composition's end toward `direction`.
    public func characterRange(
        byExtending position: UITextPosition,
        in direction: UITextLayoutDirection
    ) -> UITextRange? {
        let end = direction == .left || direction == .up ? beginningOfDocument : endOfDocument
        return textRange(from: position, to: end)
    }

    /// Left to right: what the terminal draws goes so.
    public func baseWritingDirection(
        for _: UITextPosition,
        in _: UITextStorageDirection
    ) -> NSWritingDirection {
        .leftToRight
    }

    /// The terminal keeps its own direction: nothing changes.
    public func setBaseWritingDirection(_: NSWritingDirection, for _: UITextRange) {
        // the program lays its text out
    }

    /// Where the composition shows (at the cursor's cell while nothing is composed), for the input method's candidates.
    public func firstRect(for _: UITextRange) -> CGRect {
        compositionFrame
    }

    /// The caret's rect: at the composition's end, a cell high.
    public func caretRect(for position: UITextPosition) -> CGRect {
        let frame = compositionFrame
        let offset = (position as? TerminalTextPosition)?.offset ?? 0
        let caret = offset == 0 ? frame.minX : frame.maxX
        return CGRect(x: caret, y: frame.minY, width: CompositionLayer.underline, height: frame.height)
    }

    /// No selection rects: the composition draws its own.
    public func selectionRects(for _: UITextRange) -> [UITextSelectionRect] {
        []
    }

    /// The composition's end: a point on the terminal moves nothing in it.
    public func closestPosition(to _: CGPoint) -> UITextPosition? {
        endOfDocument
    }

    /// The end of `range`: a point on the terminal moves nothing in it.
    public func closestPosition(to _: CGPoint, within range: UITextRange) -> UITextPosition? {
        range.end
    }

    /// None: a point on the terminal is no character of the composition.
    public func characterRange(at _: CGPoint) -> UITextRange? {
        nil
    }
}
