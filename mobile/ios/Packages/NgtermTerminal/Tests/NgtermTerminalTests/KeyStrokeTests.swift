import NgtermTerminal
import Testing
import UIKit

/// A hardware key as the terminal reads it from UIKit.
@MainActor
@Suite("a key stroke")
internal struct KeyStrokeTests {
    @Test("a stroke takes its key's code, modifiers and characters: an empty key's are none")
    internal func fromKey() {
        let stroke = KeyStroke(UIKey())
        #expect(stroke.code.rawValue == 0, "no key's position")
        #expect(stroke.modifiers.isEmpty)
        #expect(stroke.characters.isEmpty)
        #expect(stroke.charactersIgnoringModifiers.isEmpty)
        #expect(KeyMapping.route(stroke) == .system, "a key with no place and no text is the system's")
    }
}
