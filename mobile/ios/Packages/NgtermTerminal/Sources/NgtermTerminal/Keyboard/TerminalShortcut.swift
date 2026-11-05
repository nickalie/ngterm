import Foundation
import UIKit

/// What a hardware chord does in the app rather than in the program: upstream's Mac chords, as desktop ngterm binds
/// them on macOS (`keymap/defaults.rs`), since an iPad's keyboard has the Command key a Mac's has.
///
/// The terminal offers them as key commands with their titles, so holding ⌘ lists them; copy and paste are the
/// system's own Edit commands (⌘C, ⌘V), which the terminal answers as the first responder.
public enum TerminalShortcut: CaseIterable, Sendable {
    /// ⌘+ (or ⌘=): the text a point bigger.
    case bigger
    /// ⌘⌥→.
    case nextPane
    /// ⌘⌥↓.
    case nextSession
    /// ⌘⌥←.
    case previousPane
    /// ⌘⌥↑.
    case previousSession
    /// ⌘0: the text back to its size from Settings.
    case resetSize
    /// ⌘-: the text a point smaller.
    case smaller

    /// The chords in the order holding ⌘ lists them: the text size's, then the moves.
    public static let listed: [Self] = [
        .bigger, .smaller, .resetSize, .nextPane, .previousPane, .nextSession, .previousSession,
    ]

    // UIKit's names of the arrow keys (`UIKeyCommand.inputRightArrow` and its kin), as key commands take them
    private static let rightArrow = "UIKeyInputRightArrow"
    private static let leftArrow = "UIKeyInputLeftArrow"
    private static let downArrow = "UIKeyInputDownArrow"
    private static let upArrow = "UIKeyInputUpArrow"
    // the modifiers a chord is told by: Caps Lock and the keypad don't count
    private static let chordModifiers: UIKeyModifierFlags = [.shift, .control, .alternate, .command]

    /// The key command's input: the character, or UIKit's name of the arrow.
    public var input: String {
        switch self {
        case .bigger:
            "+"

        case .smaller:
            "-"

        case .resetSize:
            "0"

        case .nextPane:
            Self.rightArrow

        case .previousPane:
            Self.leftArrow

        case .nextSession:
            Self.downArrow

        case .previousSession:
            Self.upArrow
        }
    }

    /// The modifiers held with it.
    public var modifiers: UIKeyModifierFlags {
        switch self {
        case .bigger, .smaller, .resetSize:
            .command

        case .nextPane, .previousPane, .nextSession, .previousSession:
            [.command, .alternate]
        }
    }

    /// What holding ⌘ lists it as.
    public var title: String {
        switch self {
        case .bigger:
            String(localized: "Bigger Text")

        case .smaller:
            String(localized: "Smaller Text")

        case .resetSize:
            String(localized: "Default Text Size")

        case .nextPane:
            String(localized: "Next Pane")

        case .previousPane:
            String(localized: "Previous Pane")

        case .nextSession:
            String(localized: "Next Session")

        case .previousSession:
            String(localized: "Previous Session")
        }
    }

    /// Whether it changes the text size, which repeats while held, where a move happens once per press.
    public var isTextSize: Bool {
        modifiers == .command
    }

    // the arrow key a move goes by
    private var arrow: UIKeyboardHIDUsage? {
        switch self {
        case .bigger, .smaller, .resetSize:
            nil

        case .nextPane:
            .keyboardRightArrow

        case .previousPane:
            .keyboardLeftArrow

        case .nextSession:
            .keyboardDownArrow

        case .previousSession:
            .keyboardUpArrow
        }
    }

    /// The chord `stroke` is, if any: the text size's by the character the layout types (⌘+ is ⌘⇧= on a US layout, and
    /// ⌘= does as well), the moves by the arrow keys.
    public static func matching(_ stroke: KeyStroke) -> Self? {
        let held = stroke.modifiers.intersection(chordModifiers)
        let characters = [stroke.characters, stroke.charactersIgnoringModifiers]
        return allCases.first { shortcut in
            if let key = shortcut.arrow {
                return held == shortcut.modifiers && stroke.code == key
            }
            if shortcut == .bigger {
                let plus = characters.contains("+") || characters.contains("=")
                return plus && (held == .command || held == [.command, .shift])
            }
            return held == shortcut.modifiers && characters.contains(shortcut.input)
        }
    }

    /// The shortcut `command` (one of ``keyCommands(action:)``) stands for.
    @preconcurrency
    @MainActor
    public static func of(_ command: UIKeyCommand) -> Self? {
        let named = command.propertyList as? String
        return allCases.first { String(describing: $0) == named }
    }

    /// Its key commands, performing `action`: ⌘+ has ⌘= beside it, hidden from the list.
    @preconcurrency
    @MainActor
    public func keyCommands(action: Selector) -> [UIKeyCommand] {
        let name = String(describing: self)
        let command = UIKeyCommand(
            title: title,
            action: action,
            input: input,
            modifierFlags: modifiers,
            propertyList: name
        )
        command.wantsPriorityOverSystemBehavior = true
        command.repeatBehavior = isTextSize ? .repeatable : .nonRepeatable
        guard self == .bigger else {
            return [command]
        }
        let equal = UIKeyCommand(
            title: title,
            action: action,
            input: "=",
            modifierFlags: modifiers,
            propertyList: name,
            attributes: .hidden
        )
        equal.wantsPriorityOverSystemBehavior = true
        return [command, equal]
    }
}
