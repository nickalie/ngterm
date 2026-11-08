import NgtermCore
import NgtermCoreFakes
import NgtermTerminal
import SwiftUI
import Testing
import UIKit

@testable import NgtermFeatures

/// The session view's chords with nowhere to go (P-I5): one pane and one session, so ⌘⌥ and an arrow move nowhere and
/// stay the program's (mobile/ios/DESIGN.md: the moves are "offered once the session view gives them somewhere to go").
@MainActor
@Suite("the session view's chords with nowhere to go")
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

    // the selected pane's terminal once it is built and drawn focused; no first responder is needed for keyDown
    private static func focusedTerminal(in window: UIWindow) async throws -> TerminalView {
        var shown: [TerminalView] = []
        await Wait.until {
            window.layoutIfNeeded()
            shown = terminals(in: window).filter(\.drawsFocused)
            return shown.first != nil
        }
        return try #require(shown.first)
    }

    // the moves the terminal lists while ⌘ is held
    private static func listed(_ terminal: TerminalView) -> Set<String> {
        Set((terminal.keyCommands ?? []).map(\.title))
    }

    // presses and releases ⌘⌥ and `arrow` directly: these tests exercise routing, not UIKit's responder chain
    private static func strike(_ arrow: Arrow, on terminal: TerminalView) {
        let stroke = KeyStroke(
            code: arrow.code,
            modifiers: [.command, .alternate],
            characters: arrow.input,
            charactersIgnoringModifiers: arrow.input
        )
        #expect(terminal.keyDown(stroke))
        #expect(terminal.keyUp(stroke))
    }

    private static func pressed(_ pane: FakePane) -> [PaneKey] {
        pane.inputs.compactMap { input in
            if case .key(let key) = input, key.action == .press {
                return key
            }
            return nil
        }
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
            Self.strike(arrow, on: terminal)
        }
        let mods = KeyMapping.super | KeyMapping.alt
        #expect(Self.pressed(pane) == Self.arrows.map { KeyMapping.named($0.name, mods: mods) })
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
        let terminal = try await Self.focusedTerminal(in: window)
        let pane = try #require(terminal.pane as? FakePane)
        #expect(SessionModel(graph: world.graph, key: SessionWorld.key).focusedPane == "P1")
        Self.strike(Self.arrows[0], on: terminal)
        await Wait.until { world.graph.panes.focus[SessionWorld.key] == "P2" }
        #expect(Self.pressed(pane).isEmpty, "the move stays out of the program")
    }

    @Test("when the split returns to one pane, its moves disappear and the arrows reach the program again")
    internal func paneRemoved() async throws {
        defer { world.remove() }
        let window = try await showSession()
        defer { window.isHidden = true }
        await world.stream(SessionWorld.split("vertical"))
        let split = try await Self.focusedTerminal(in: window)
        await Wait.until { Self.listed(split).isSuperset(of: Self.paneMoves) }

        await world.stream(SessionWorld.layout(["P1"], axis: nil, shown: true))
        let terminal = try await Self.focusedTerminal(in: window)
        await Wait.until { Self.listed(terminal).isDisjoint(with: Self.paneMoves) }
        let pane = try #require(terminal.pane as? FakePane)
        for arrow in Self.arrows.prefix(2) {
            Self.strike(arrow, on: terminal)
        }
        let mods = KeyMapping.super | KeyMapping.alt
        #expect(Self.pressed(pane) == Self.arrows.prefix(2).map { KeyMapping.named($0.name, mods: mods) })
    }

    @Test("attaching another session installs its moves; detaching it returns their arrows to the program")
    internal func sessionChanged() async throws {
        defer { world.remove() }
        var opened: [SessionKey] = []
        let window = try await showSession { opened.append($0) }
        defer { window.isHidden = true }
        let terminal = try await Self.focusedTerminal(in: window)
        let pane = try #require(terminal.pane as? FakePane)
        #expect(Self.listed(terminal).isDisjoint(with: Self.sessionMoves))

        let sessions = [fakeSession(id: "s1", name: "api"), fakeSession(id: "s2", name: "web")]
        world.core.sessions["m1"] = sessions
        await world.graph.connections.listSessions(machineId: "m1")
        await world.test.settled()
        try await world.graph.connections.attach(machineId: "m1", sessionId: "s2")
        await Wait.until { Self.listed(terminal).isSuperset(of: Self.sessionMoves) }
        #expect(Self.listed(terminal).isDisjoint(with: Self.paneMoves))
        for arrow in Self.arrows.suffix(2) {
            Self.strike(arrow, on: terminal)
        }
        let next = SessionKey(machineId: "m1", sessionId: "s2")
        #expect(opened == [next, next], "both directions wrap to the other session")
        #expect(Self.pressed(pane).isEmpty, "session moves stay out of the program")

        world.graph.connections.detach(machineId: "m1", sessionId: "s2")
        await Wait.until { Self.listed(terminal).isDisjoint(with: Self.sessionMoves) }
        for arrow in Self.arrows.suffix(2) {
            Self.strike(arrow, on: terminal)
        }
        let mods = KeyMapping.super | KeyMapping.alt
        #expect(Self.pressed(pane) == Self.arrows.suffix(2).map { KeyMapping.named($0.name, mods: mods) })
        #expect(opened == [next, next], "no stale move runs")
    }

    // the only attached session, showing its one pane `P1` with a hardware keyboard, in a phone-sized window
    private func showSession(
        onOpen: @escaping @MainActor (SessionKey) -> Void = { _ in Self.ignore() }
    ) async throws -> UIWindow {
        try await world.attach()
        await world.stream(SessionWorld.layout(["P1"], axis: nil, shown: true))
        let keyboard = TerminalKeyboard(hardware: HardwareKeyboard(fixed: true))
        let screen = SessionScreen(
            graph: world.graph,
            session: SessionWorld.key,
            actions: SessionActions(close: Self.ignore, open: onOpen, openInNewWindow: nil),
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
