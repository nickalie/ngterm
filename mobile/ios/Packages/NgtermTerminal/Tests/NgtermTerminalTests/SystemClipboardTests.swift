import NgtermTerminal
import Testing
import UIKit

/// The system's clipboard as the terminal uses it: the general pasteboard, as text.
@MainActor
@Suite("the system clipboard")
internal struct SystemClipboardTests {
    @Test("what the terminal copies is on the general pasteboard, and an empty copy empties it")
    internal func roundTrip() {
        let clipboard = SystemClipboard()
        clipboard.write("ngterm copied this")
        #expect(clipboard.hasText)
        #expect(UIPasteboard.general.hasStrings)
        clipboard.write(nil)
        #expect(!clipboard.hasText)
        #expect(!UIPasteboard.general.hasStrings)
    }
}
