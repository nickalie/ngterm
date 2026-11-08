import NgtermCore
import NgtermCoreFakes
import NgtermTerminal
import SwiftUI
import Testing
import UIKit

@testable import NgtermFeatures

/// The session view's chords with nowhere to go (P-I5): one pane and one session, so ⌘⌥ and an arrow move nowhere and
/// stay the program's (mobile/ios/DESIGN.md: the moves are "offered once the session view gives them somewhere to go").
///
/// One test at a time: each shows its own window, and the keyboard's focus is the key window's.
@MainActor
@Suite("the session view's chords with nowhere to go", .serialized)
internal struct ChordsNowhereToGoTests {
    // an arrow key: its place, what UIKit calls it, and the program's name for it
    private struct Arrow {
        let code: UIKeyboardHIDUsage
        let input: String
        let name: String
    }

    private static let arrows = [
        Arrow(code: .keyboardRightArrow, input: "UIKeyInputRightArrow", name: "arrow_right"),
        Arrow(code: .keyboardLeftArrow, input: "UIKeyInputLeftArrow", name: "arrow_left"),
        Arrow(code: .keyboardDownArrow, input: "UIKeyInputDownArrow", name: "arrow_down"),
        Arrow(code: .keyboardUpArrow, input: "UIKeyInputUpArrow", name: "arrow_up"),
    ]
    private static let paneMoves: Set = ["Next Pane", "Previous Pane"]
    private static let sessionMoves: Set = ["Next Session", "Previous Session"]

    // what the session view's actions do here: nothing
    private static let ignore: @MainActor () -> Void = {
        // nothing to close or open
    }

    private let world = SessionWorld()

    private static func terminals(in view: UIView) -> [TerminalView] {
        if let terminal = view as? TerminalView {
            return [terminal]
        }
        return view.subviews.flatMap(terminals)
    }

    // the selected pane's terminal, once it has the keyboard's focus
    private static func focusedTerminal(in window: UIWindow) async throws -> TerminalView {
        var shown: [TerminalView] = []
        await Wait.until {
            window.layoutIfNeeded()
            shown = terminals(in: window)
            return shown.first?.drawsFocused == true
        }
        return try #require(shown.first)
    }

    // the moves the terminal lists while ⌘ is held
    private static func listed(_ terminal: TerminalView) -> Set<String> {
        Set((terminal.keyCommands ?? []).map(\.title))
    }

    @Test("with one pane and one session, ⌘⌥ and the arrows reach the program, and no move is listed")
    internal func nowhereToGo() async throws {
        defer { world.remove() }
        let window = try await showSession()
        defer { window.isHidden = true }
        let terminal = try await Self.focusedTerminal(in: window)
        let pane = try #require(terminal.pane as? FakePane)

        #expect(Self.listed(terminal).isDisjoint(with: Self.paneMoves.union(Self.sessionMoves)))
        for arrow in Self.arrows {
            let stroke = KeyStroke(
                code: arrow.code,
                modifiers: [.command, .alternate],
                characters: arrow.input,
                charactersIgnoringModifiers: arrow.input
            )
            _ = terminal.keyDown(stroke)
            _ = terminal.keyUp(stroke)
        }
        let pressed = pane.inputs.compactMap { input -> PaneKey? in
            if case .key(let key) = input, key.action == .press {
                return key
            }
            return nil
        }
        let mods = KeyMapping.super | KeyMapping.alt
        #expect(pressed == Self.arrows.map { KeyMapping.named($0.name, mods: mods) })
    }

    @Test("a second pane gives the pane moves somewhere to go, and the session moves still have none")
    internal func secondPane() async throws {
        defer { world.remove() }
        let window = try await showSession()
        defer { window.isHidden = true }
        _ = try await Self.focusedTerminal(in: window)

        await world.stream(SessionWorld.split("vertical"))
        var listed: Set<String> = []
        await Wait.until {
            window.layoutIfNeeded()
            listed = Self.terminals(in: window).first(where: \.drawsFocused).map(Self.listed) ?? []
            return listed.isSuperset(of: Self.paneMoves)
        }
        #expect(listed.isSuperset(of: Self.paneMoves))
        #expect(listed.isDisjoint(with: Self.sessionMoves))
    }

    // the only attached session, showing its one pane `P1` with a hardware keyboard, in a phone-sized window
    private func showSession() async throws -> UIWindow {
        try await world.attach()
        await world.stream(SessionWorld.layout(["P1"], axis: nil, shown: true))
        let keyboard = TerminalKeyboard(hardware: HardwareKeyboard(fixed: true))
        let screen = SessionScreen(
            graph: world.graph,
            session: SessionWorld.key,
            actions: SessionActions(close: Self.ignore, open: { _ in Self.ignore() }, openInNewWindow: nil),
            keyboard: keyboard
        )
        let host = UIHostingController(rootView: NavigationStack { screen })
        let made = (UIWindow.self as UIView.Type).init(frame: CGRect(x: 0, y: 0, width: 400, height: 800))
        let window = try #require(made as? UIWindow)
        window.rootViewController = host
        window.isHidden = false
        return window
    }
}
