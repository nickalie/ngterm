import NgtermCoreFakes
import NgtermDesign
import NgtermTerminal
import Testing
import UIKit

/// The keyboard's focus following the session view's selection, in a window of the app's scene (the first responder
/// needs one): the terminal selected next takes the focus the last one had, with its soft keyboard, whether it is on
/// screen already (a split) or comes on as the other goes (a phone's one pane).
@MainActor
@Suite("the focus handed over between panes")
internal struct TerminalHandoverTests {
    private static let frame = CGRect(x: 0, y: 0, width: 200, height: 100)

    private static func window() throws -> UIWindow {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let window = UIWindow(windowScene: try #require(scenes.first))
        window.frame = CGRect(x: 0, y: 0, width: 400, height: 400)
        window.makeKeyAndVisible()
        return window
    }

    private static func terminal(_ keyboard: TerminalKeyboard, focused: Bool) -> TerminalView {
        let view = TerminalView(pane: FakePane(), palette: .copper, textSize: 13, keyboard: keyboard)
        view.frame = frame
        view.drawsFocused = focused
        return view
    }

    @Test("in a split, the pane selected next takes the focus and the soft keyboard; a touch selects")
    internal func split() throws {
        let window = try Self.window()
        defer { window.isHidden = true }
        let keyboard = TerminalKeyboard(hardware: HardwareKeyboard(fixed: false))
        let first = Self.terminal(keyboard, focused: true)
        let second = Self.terminal(keyboard, focused: false)
        var selected: [String] = []
        first.onSelect = { selected.append("first") }
        second.onSelect = { selected.append("second") }
        window.addSubview(first)
        window.addSubview(second)

        first.focus(showingKeyboard: true)
        #expect(first.isFirstResponder)
        #expect(selected == ["first"], "taking the focus selects the pane")
        keyboard.handOver()
        first.drawsFocused = false
        second.drawsFocused = true
        #expect(second.isFirstResponder)
        #expect(second.softKeyboardWanted)
        #expect(keyboard.target === second)

        // with nothing focused, a new selection takes no focus
        _ = second.resignFirstResponder()
        keyboard.handOver()
        second.drawsFocused = false
        first.drawsFocused = true
        #expect(!first.isFirstResponder)
    }

    @Test("on a phone, the pane that comes on as the other goes takes the focus, without the soft keyboard if closed")
    internal func onePane() throws {
        let window = try Self.window()
        defer { window.isHidden = true }
        let keyboard = TerminalKeyboard(hardware: HardwareKeyboard(fixed: false))
        let first = Self.terminal(keyboard, focused: true)
        window.addSubview(first)
        first.focus()
        #expect(first.isFirstResponder)

        keyboard.handOver()
        let second = Self.terminal(keyboard, focused: true)
        first.removeFromSuperview()
        window.addSubview(second)
        #expect(second.isFirstResponder)
        #expect(!second.softKeyboardWanted)

        // taken once: a terminal coming on later takes nothing
        let third = Self.terminal(keyboard, focused: true)
        window.addSubview(third)
        #expect(!third.isFirstResponder)
    }
}
