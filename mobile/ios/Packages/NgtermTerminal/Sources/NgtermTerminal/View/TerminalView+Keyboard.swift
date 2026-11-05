import NgtermCore
import UIKit

extension TerminalView {
    /// The app's chords (``TerminalShortcut``) as key commands: the text size's always, a move while the session view
    /// gives it an action.
    internal var shortcutCommands: [UIKeyCommand] {
        TerminalShortcut.listed
            .filter { $0.isTextSize || keyboard.shortcuts.action(for: $0) != nil }
            .flatMap { $0.keyCommands(action: #selector(runKeyCommand)) }
    }

    /// The terminal became the first responder (`became`): every key goes here now.
    internal func tookKeyboard(_ became: Bool) -> Bool {
        if became {
            keyboard.target = self
            keyboard.refreshBar()
            onSelect?()
        }
        return became
    }

    /// The terminal drawn focused (the selected pane) in a window takes the focus the last selected one handed over,
    /// opening the soft keyboard if that one had it open.
    internal func takeHandover() {
        guard drawsFocused, window != nil, let handover = keyboard.takeHandover() else {
            return
        }
        focus(showingKeyboard: handover == .withKeyboard)
    }

    /// The terminal let the first responder go (`resigned`), or isn't it: the keys held are let go, the keys go
    /// nowhere, and the next focus opens the soft keyboard only if it is a tap.
    internal func letKeyboardGo(_ resigned: Bool) -> Bool {
        if resigned || !isFirstResponder {
            softKeyboardWanted = false
            for key in heldKeys.letGoAll() {
                send(key: KeyMapping.key(key, as: .release))
            }
            if keyboard.target === self {
                keyboard.target = nil
            }
        }
        return resigned
    }

    /// The presses of `presses` that `taken` doesn't take, which go on to the text system.
    internal func untaken(_ presses: Set<UIPress>, by taken: (KeyStroke) -> Bool) -> Set<UIPress> {
        presses.filter { press in
            press.key.map { !taken(KeyStroke($0)) } ?? true
        }
    }

    /// A hardware key went down: whether it is taken here, rather than left to the text system.
    ///
    /// An app chord runs (where no key command ran it already); while an input method composes, every key is its own;
    /// a special key or a chord goes to the program with the sticky modifiers, repeating while held; a modifier key
    /// goes to the program and the system both; what types plainly is the text system's, which types it through
    /// ``insertText(_:)``.
    public func keyDown(_ stroke: KeyStroke) -> Bool {
        if let shortcut = TerminalShortcut.matching(stroke), run(shortcut) {
            heldKeys.hold(stroke.code, as: .chord)
            return true
        }
        guard !textInput.isComposing else {
            return false
        }
        switch KeyMapping.route(stroke) {
        case .text, .system:
            return false

        case .modifier:
            let key = KeyMapping.key(stroke)
            send(key: key)
            heldKeys.hold(stroke.code, as: .program(key, shared: true))
            return false

        case .key:
            var key = KeyMapping.key(stroke)
            key.mods |= keyboard.takeSticky()
            send(key: key)
            heldKeys.hold(stroke.code, as: .program(key, shared: false))
            heldKeys.repeatKey(stroke.code) { [weak self] in
                self?.send(key: KeyMapping.key(key, as: .repeat))
            }
            return true
        }
    }

    /// A hardware key was let go: whether it was taken here.
    ///
    /// The program hears of a key it got, with the modifiers it was pressed with.
    public func keyUp(_ stroke: KeyStroke) -> Bool {
        switch heldKeys.letGo(stroke.code) {
        case .chord:
            return true

        case .program(let key, let shared):
            send(key: KeyMapping.key(key, as: .release))
            return !shared

        case nil:
            return false
        }
    }

    /// Runs `shortcut`: a text size step, or the session view's move; false for a move it gave no action, which is the
    /// program's chord then.
    @discardableResult
    public func run(_ shortcut: TerminalShortcut) -> Bool {
        let sizes = TerminalPointer.textSizes
        switch shortcut {
        case .bigger:
            textSize = min(textSize + 1, sizes.upperBound)

        case .smaller:
            textSize = max(textSize - 1, sizes.lowerBound)

        case .resetSize:
            textSize = defaultTextSize

        case .nextPane, .previousPane, .nextSession, .previousSession:
            guard let action = keyboard.shortcuts.action(for: shortcut) else {
                return false
            }
            action()
        }
        return true
    }

    /// Closes the soft keyboard, keeping the focus: the extra keys stay at the screen's foot.
    public func hideSoftKeyboard() {
        guard softKeyboardWanted else {
            return
        }
        softKeyboardWanted = false
        reloadInputViews()
        keyboard.refreshBar()
    }

    /// Sends `key` to the program; a pane with no terminal now drops it.
    internal func send(key: PaneKey) {
        _ = try? pane.key(key: key)
    }

    /// Types `text` into the program.
    internal func send(text: String) {
        _ = try? pane.typeText(text: text)
    }

    /// Presses `key` and lets it go.
    internal func pressKey(_ key: PaneKey) {
        send(key: key)
        send(key: KeyMapping.key(key, as: .release))
    }

    @objc
    private func runKeyCommand(_ command: UIKeyCommand) {
        if let shortcut = TerminalShortcut.of(command) {
            run(shortcut)
        }
    }
}
