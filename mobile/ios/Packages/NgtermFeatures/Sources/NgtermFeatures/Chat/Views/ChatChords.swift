import NgtermTerminal
import SwiftUI

/// The session view's chords from inside a chat (⌘⌥ arrows), whose field, not a terminal, has the keyboard.
internal struct ChatChords: View {
    internal let keyboard: TerminalKeyboard

    internal var body: some View {
        ZStack {
            chord(.rightArrow) { $0.nextPane }
            chord(.leftArrow) { $0.previousPane }
            chord(.downArrow) { $0.nextSession }
            chord(.upArrow) { $0.previousSession }
        }
        .frame(width: 0, height: 0)
        .opacity(0)
        .accessibilityHidden(true)
    }

    // the move as the session view set it when the chord is pressed
    private func chord(
        _ key: KeyEquivalent,
        _ move: @escaping @MainActor (TerminalShortcuts) -> (@MainActor () -> Void)?
    ) -> some View {
        Button {
            move(keyboard.shortcuts)?()
        } label: {
            EmptyView()
        }
        .keyboardShortcut(key, modifiers: [.command, .option])
    }
}
