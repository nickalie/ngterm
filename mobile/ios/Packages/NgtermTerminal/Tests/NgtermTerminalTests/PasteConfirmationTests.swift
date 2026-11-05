import NgtermTerminal
import Testing
import UIKit

/// The confirmation before an unsafe paste: what it shows of the text, and its two answers.
@MainActor
@Suite("the paste confirmation")
internal struct PasteConfirmationTests {
    @Test("it shows the text's first lines as they are, its control characters as symbols")
    internal func preview() {
        #expect(PasteConfirmation.preview(of: "make\nmake install\n") == "make\nmake install\n")
        #expect(PasteConfirmation.preview(of: "a\r\nb") == "a\nb")
        #expect(PasteConfirmation.preview(of: "say \u{1B}[31mred\tok\u{7F}") == "say \u{241B}[31mred\u{2409}ok\u{2421}")
    }

    @Test("it cuts long lines and many lines, and says so")
    internal func cuts() {
        let long = String(repeating: "x", count: PasteConfirmation.previewWidth + 5)
        let cut = String(repeating: "x", count: PasteConfirmation.previewWidth) + "…"
        #expect(PasteConfirmation.preview(of: long) == cut)
        let lines = (1...9).map { "line \($0)" }.joined(separator: "\n")
        let shown = PasteConfirmation.preview(of: lines).split(separator: "\n", omittingEmptySubsequences: false)
        #expect(shown.count == PasteConfirmation.previewLines + 1)
        #expect(shown.first == "line 1")
        #expect(shown.last == "…")
    }

    @Test("the alert asks with the text under what it can do, Cancel the safe answer and Paste the other")
    internal func alert() {
        let alert = PasteConfirmation.alert(for: "rm -rf build\n") {
            // a test presses nothing
        } onCancel: {
            // a test presses nothing
        }
        #expect(alert.title == "Paste text that can run a command?")
        #expect(alert.message?.hasSuffix("\n\nrm -rf build\n") == true)
        #expect(alert.actions.map(\.title) == ["Cancel", "Paste"])
        #expect(alert.actions.map(\.style) == [.cancel, .default])
    }
}
