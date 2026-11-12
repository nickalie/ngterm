import NgtermCoreFakes
import NgtermTerminal
import Testing
import UIKit

/// The composition as an input method sees it through `UITextInput`: its positions and spans, edits within it, and
/// where its caret and candidates go.
@MainActor
@Suite("the terminal's text input")
internal struct TerminalTextInputTests {
    // a rig whose view is its keyboard's target, as the first responder would be
    private static func rig() -> TerminalRig {
        let rig = TerminalRig(clipboard: nil)
        rig.view.keyboard.target = rig.view
        return rig
    }

    // the place `offset` units into the composition
    private static func at(_ offset: Int, in view: TerminalView) throws -> UITextPosition {
        try #require(view.position(from: view.beginningOfDocument, offset: offset))
    }

    // the span from `start` to `end` units into the composition
    private static func span(_ start: Int, _ end: Int, in view: TerminalView) throws -> UITextRange {
        try #require(view.textRange(from: at(start, in: view), to: at(end, in: view)))
    }

    // where `position` is, in units from the composition's start
    private static func offset(of position: UITextPosition, in view: TerminalView) -> Int {
        view.offset(from: view.beginningOfDocument, to: position)
    }

    @Test("each key the soft keyboard types reaches the program as it goes down, with nothing held for the word")
    internal func eachKey() {
        let rig = Self.rig()
        let view = rig.view
        var sent: [FakeInput] = []
        for key in ["/", "c", "l"] {
            view.insertText(key)
            sent.append(.typed(key))
            #expect(rig.inputs == sent)
            #expect(view.markedTextRange == nil)
            #expect(view.composition.isEmpty)
        }
    }

    @Test("the input method's selection moves within the composition, and a deletion takes what it selects")
    internal func selection() throws {
        let rig = Self.rig()
        let view = rig.view
        view.setMarkedText("abcd", selectedRange: NSRange(location: 4, length: 0))
        view.selectedTextRange = try Self.span(1, 3, in: view)
        let selected = try #require(view.selectedTextRange)
        #expect(Self.offset(of: selected.start, in: view) == 1)
        #expect(Self.offset(of: selected.end, in: view) == 3)
        #expect(view.text(in: selected) == "bc")
        // a range that isn't the terminal's moves nothing
        view.selectedTextRange = UITextRange()
        #expect(view.selectedTextRange.map { Self.offset(of: $0.start, in: view) } == 1)
        view.deleteBackward()
        #expect(view.composition == "ad")
        #expect(rig.inputs.isEmpty, "a deletion within the composition is its own")
    }

    @Test("a replacement within the composition stays composed; with nothing composed it is typed")
    internal func replacement() throws {
        let rig = Self.rig()
        let view = rig.view
        view.setMarkedText("abc", selectedRange: NSRange(location: 3, length: 0))
        view.replace(try Self.span(1, 2, in: view), withText: "XY")
        #expect(view.composition == "aXYc")
        #expect(rig.inputs.isEmpty)
        view.unmarkText()
        let gone = try Self.span(0, 0, in: view)
        view.replace(gone, withText: "ls")
        #expect(rig.inputs == [.typed("aXYc"), .typed("ls")])
        #expect(view.composition.isEmpty)
    }

    @Test(
        "a deletion with the caret inside a composed character takes that whole character",
        arguments: ["😀", "e\u{301}", "👩‍💻"], [false, true]
    )
    internal func deletionInsideCharacter(_ character: String, movingSelection: Bool) throws {
        let rig = Self.rig()
        let view = rig.view
        let text = "a" + character + "z"
        view.setMarkedText(text, selectedRange: NSRange(location: movingSelection ? text.utf16.count : 2, length: 0))
        if movingSelection {
            view.selectedTextRange = try Self.span(2, 2, in: view)
        }
        view.deleteBackward()
        #expect(view.composition == "az")
        let selected = try #require(view.selectedTextRange)
        #expect(Self.offset(of: selected.start, in: view) == 1)
        #expect(selected.isEmpty)
        #expect(rig.inputs.isEmpty)
    }

    @Test(
        "an insertion inside a composed character goes before it, with its caret after the inserted text",
        arguments: ["😀", "e\u{301}", "👩‍💻"]
    )
    internal func replacementInsideCharacter(_ character: String) throws {
        let rig = Self.rig()
        let view = rig.view
        view.setMarkedText(character, selectedRange: NSRange(location: character.utf16.count, length: 0))
        view.replace(try Self.span(1, 1, in: view), withText: "x")
        #expect(view.composition == "x" + character)
        let selected = try #require(view.selectedTextRange)
        #expect(Self.offset(of: selected.start, in: view) == 1)
        #expect(selected.isEmpty)
        view.deleteBackward()
        #expect(view.composition == character, "backspace takes only the inserted x")
        #expect(rig.inputs.isEmpty)
    }

    @Test(
        "a replacement that selects part of a composed character takes the whole character",
        arguments: ["😀", "e\u{301}", "👩‍💻"]
    )
    internal func replacementOfPartialCharacter(_ character: String) throws {
        let rig = Self.rig()
        let view = rig.view
        view.setMarkedText("a" + character + "z", selectedRange: NSRange(location: 0, length: 0))
        view.replace(try Self.span(2, 3, in: view), withText: "XY")
        #expect(view.composition == "aXYz")
        let selected = try #require(view.selectedTextRange)
        #expect(Self.offset(of: selected.start, in: view) == 3)
        #expect(selected.isEmpty)
        #expect(rig.inputs.isEmpty)
    }

    @Test("the text of a span the composition no longer has is none")
    internal func textPastComposition() throws {
        let view = Self.rig().view
        view.setMarkedText("abc", selectedRange: NSRange(location: 3, length: 0))
        let whole = try Self.span(0, 3, in: view)
        #expect(view.text(in: whole) == "abc")
        view.unmarkText()
        #expect(view.text(in: whole) == nil)
        #expect(view.text(in: UITextRange()) == nil)
    }

    @Test("positions stay within the composition: none past its ends, up and down go to them")
    internal func positions() throws {
        let view = Self.rig().view
        view.setMarkedText("abcd", selectedRange: NSRange(location: 2, length: 0))
        let start = view.beginningOfDocument
        let end = view.endOfDocument
        #expect(view.offset(from: start, to: end) == 4)
        #expect(view.position(from: start, offset: -1) == nil)
        #expect(view.position(from: end, offset: 1) == nil)
        let two = try #require(view.position(from: start, in: .right, offset: 2))
        #expect(Self.offset(of: two, in: view) == 2)
        let one = try #require(view.position(from: two, in: .left, offset: 1))
        #expect(Self.offset(of: one, in: view) == 1)
        #expect(view.position(from: two, in: .left, offset: 3) == nil)
        let top = try #require(view.position(from: two, in: .up, offset: 1))
        let bottom = try #require(view.position(from: two, in: .down, offset: 1))
        #expect(view.compare(top, to: start) == .orderedSame)
        #expect(view.compare(bottom, to: end) == .orderedSame)
        #expect(view.compare(one, to: two) == .orderedAscending)
        #expect(view.compare(two, to: one) == .orderedDescending)
        // a position that isn't the terminal's is no place in it
        #expect(view.position(from: UITextPosition(), offset: 0) == nil)
        #expect(view.textRange(from: UITextPosition(), to: end) == nil)
        #expect(view.textRange(from: start, to: UITextPosition()) == nil)
    }

    @Test("a span's far end is its start leftward and up, its end rightward and down; extending runs to the ends")
    internal func directions() throws {
        let view = Self.rig().view
        view.setMarkedText("abcd", selectedRange: NSRange(location: 2, length: 0))
        let middle = try Self.span(1, 3, in: view)
        let far = { (direction: UITextLayoutDirection) in
            view.position(within: middle, farthestIn: direction).map { Self.offset(of: $0, in: view) }
        }
        #expect(far(.left) == 1)
        #expect(far(.up) == 1)
        #expect(far(.right) == 3)
        #expect(far(.down) == 3)
        let two = try Self.at(2, in: view)
        let back = try #require(view.characterRange(byExtending: two, in: .left))
        #expect(view.text(in: back) == "ab")
        let upward = try #require(view.characterRange(byExtending: two, in: .up))
        #expect(view.text(in: upward) == "ab")
        let forward = try #require(view.characterRange(byExtending: two, in: .right))
        #expect(view.text(in: forward) == "cd")
    }

    @Test("the candidates go where the composition shows, the caret at its ends, and no point moves within it")
    internal func geometry() throws {
        let view = Self.rig().view
        view.setMarkedText("abc", selectedRange: NSRange(location: 3, length: 0))
        let frame = view.compositionFrame
        let whole = try Self.span(0, 3, in: view)
        #expect(view.firstRect(for: whole) == frame)
        let first = view.caretRect(for: view.beginningOfDocument)
        let last = view.caretRect(for: view.endOfDocument)
        #expect(first.minX == frame.minX)
        #expect(last.minX == frame.maxX)
        #expect(first.height == frame.height && last.minY == frame.minY)
        #expect(view.selectionRects(for: whole).isEmpty, "the composition draws its own")
        let somewhere = CGPoint(x: frame.midX, y: frame.midY)
        #expect(view.closestPosition(to: somewhere).map { Self.offset(of: $0, in: view) } == 3)
        let middle = try Self.span(1, 2, in: view)
        #expect(view.closestPosition(to: somewhere, within: middle).map { Self.offset(of: $0, in: view) } == 2)
        #expect(view.characterRange(at: somewhere) == nil)
        #expect(view.textInputView === view)
    }

    @Test("the text goes left to right, whatever the system asks")
    internal func writingDirection() throws {
        let view = Self.rig().view
        view.setMarkedText("ab", selectedRange: NSRange(location: 2, length: 0))
        let whole = try Self.span(0, 2, in: view)
        view.setBaseWritingDirection(.rightToLeft, for: whole)
        #expect(view.baseWritingDirection(for: view.beginningOfDocument, in: .forward) == .leftToRight)
        #expect(view.baseWritingDirection(for: view.endOfDocument, in: .backward) == .leftToRight)
    }

    @Test("the style the system asks the composition be drawn in is kept, and cleared")
    internal func markedStyle() {
        let view = Self.rig().view
        #expect(view.markedTextStyle?.isEmpty == true)
        view.markedTextStyle = [.foregroundColor: UIColor.red]
        #expect(view.markedTextStyle?[.foregroundColor] as? UIColor == .red)
        view.markedTextStyle = nil
        #expect(view.markedTextStyle?.isEmpty == true)
    }
}
