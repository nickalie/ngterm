import UIKit

// VoiceOver reads the terminal line by line, as it reads a book's page: each row of the screen is a line, framed on
// its row, and the page is the screen's text down to its last row with text.
extension TerminalView: UIAccessibilityReadingContent {
    /// The row under `point` (in screen coordinates), or `NSNotFound` off the screen's rows.
    public func accessibilityLineNumber(for point: CGPoint) -> Int {
        let local = convert(point, from: nil)
        guard bounds.contains(local), font.cellHeight > 0 else {
            return NSNotFound
        }
        let row = Int(local.y / font.cellHeight)
        return row < screen.rowCount ? row : NSNotFound
    }

    /// The text of row `lineNumber` as drawn, trailing blanks trimmed; `nil` off the screen's rows.
    public func accessibilityContent(forLineNumber lineNumber: Int) -> String? {
        guard (0..<screen.rowCount).contains(lineNumber) else {
            return nil
        }
        return screen.text(lineNumber)
    }

    /// The frame of the row `lineNumber`, in screen coordinates; empty off the screen's rows.
    public func accessibilityFrame(forLineNumber lineNumber: Int) -> CGRect {
        guard (0..<screen.rowCount).contains(lineNumber) else {
            return .zero
        }
        let row = CGRect(x: 0, y: font.top(lineNumber), width: bounds.width, height: font.cellHeight)
        return UIAccessibility.convertToScreenCoordinates(row, in: self)
    }

    /// The screen's text, a line per row, down to its last row with text.
    public func accessibilityPageContent() -> String? {
        var rows = (0..<screen.rowCount).map(screen.text)
        while rows.last?.isEmpty == true {
            rows.removeLast()
        }
        return rows.joined(separator: "\n")
    }
}
