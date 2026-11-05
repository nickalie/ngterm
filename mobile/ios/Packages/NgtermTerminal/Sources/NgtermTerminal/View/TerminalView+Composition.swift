import CoreText
import UIKit

extension TerminalView {
    /// Where the composition shows: at the cursor's cell, as wide as its text, moved left as far as it needs to fit the
    /// view; a cell wide at the cursor while nothing is composed.
    public var compositionFrame: CGRect {
        let cursor = screen.header?.cursor
        let width = textInput.isComposing ? Self.width(of: compositionLine()) : font.cellWidth
        let left = max(min(font.left(cursor?.column ?? 0), bounds.width - width), 0)
        return CGRect(x: left, y: font.top(cursor?.row ?? 0), width: width, height: font.cellHeight)
    }

    /// What the input method composes now; empty while nothing is.
    public var composition: String {
        textInput.composed
    }

    private static func width(of line: CTLine) -> CGFloat {
        CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil)).rounded(.up)
    }

    /// A key from elsewhere than the soft keyboard is coming (an extra key, a sticky tap), or the terminal lets the
    /// keyboard go: what is composed goes to the program first, under the sticky modifiers as they stand, and the input
    /// method is told it is done with.
    public func commitComposition() {
        guard textInput.isComposing else {
            return
        }
        let delegate = textInput.delegate
        delegate?.selectionWillChange(self)
        delegate?.textWillChange(self)
        let text = textInput.take()
        placeComposition()
        delegate?.textDidChange(self)
        delegate?.selectionDidChange(self)
        keyboard.type(text, to: self)
    }

    /// Draws the composition where it shows now, in the palette's colors; hides it while nothing is composed.
    internal func placeComposition() {
        let shown = textInput.layer
        guard textInput.isComposing else {
            shown.isHidden = true
            shown.line = nil
            return
        }
        shown.frame = compositionFrame
        shown.line = compositionLine()
        shown.baseline = font.baseline
        shown.ground = palette.background.uiColor.cgColor
        shown.ink = palette.foreground.uiColor.cgColor
        shown.mark = palette.cursor.uiColor.cgColor
        shown.contentsScale = scale
        shown.isHidden = false
        shown.setNeedsDisplay()
    }

    // the composition shaped in the terminal's regular face, its color taken from the context
    private func compositionLine() -> CTLine {
        let attributes: [NSAttributedString.Key: Any] = [
            NSAttributedString.Key(kCTFontAttributeName as String): font.face(for: []),
            NSAttributedString.Key(kCTForegroundColorFromContextAttributeName as String): true,
        ]
        return CTLineCreateWithAttributedString(NSAttributedString(string: textInput.composed, attributes: attributes))
    }
}
