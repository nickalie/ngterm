import Foundation

/// A key of the extra keys: its legend, how VoiceOver names it, and what it does (mobile/android's `ExtraKey`).
public enum ExtraKey: CaseIterable, Sendable {
    /// The sticky Alt.
    case alt
    /// The sticky Ctrl.
    case control
    /// Down arrow.
    case downArrow
    /// End.
    case end
    /// Esc.
    case escape
    /// Home.
    case home
    /// Opens or closes the soft keyboard: a phone's has no key of its own that closes it.
    case keyboard
    /// Left arrow.
    case leftArrow
    /// Page Down.
    case pageDown
    /// Page Up.
    case pageUp
    /// `|`.
    case pipe
    /// Right arrow.
    case rightArrow
    /// `/`.
    case slash
    /// Tab.
    case tab
    /// `~`.
    case tilde
    /// Up arrow.
    case upArrow

    /// The bar's two lines on a phone held upright: the arrows an inverted T at the end with Home and End over Left and
    /// Right, Ctrl and Alt by Tab where the thumb rests at the start, the keyboard's key between them.
    public static let twoRows: [[Self]] = [
        [.escape, .slash, .pipe, .tilde, .home, .upArrow, .end, .pageUp],
        [.tab, .control, .alt, .keyboard, .leftArrow, .downArrow, .rightArrow, .pageDown],
    ]

    /// The bar in one line once every key has its 44 points: the keys that hold and send first, the moves after them,
    /// the keyboard's key at the end.
    public static let oneRow: [Self] = [
        .escape, .tab, .control, .alt, .slash, .pipe, .tilde, .home, .end, .pageUp, .pageDown, .leftArrow, .upArrow,
        .downArrow, .rightArrow, .keyboard,
    ]

    /// What it does.
    public var action: ExtraKeyAction {
        switch self {
        case .escape:
            .press("escape", repeats: false)

        case .tab:
            .press("tab", repeats: false)

        case .control:
            .hold(.control)

        case .alt:
            .hold(.alt)

        case .slash:
            .type("/")

        case .pipe:
            .type("|")

        case .tilde:
            .type("~")

        case .home:
            .press("home", repeats: false)

        case .end:
            .press("end", repeats: false)

        case .pageUp:
            .press("page_up", repeats: true)

        case .pageDown:
            .press("page_down", repeats: true)

        case .leftArrow:
            .press("arrow_left", repeats: true)

        case .upArrow:
            .press("arrow_up", repeats: true)

        case .downArrow:
            .press("arrow_down", repeats: true)

        case .rightArrow:
            .press("arrow_right", repeats: true)

        case .keyboard:
            .softKeyboard
        }
    }

    /// Its accessibility identifier.
    public var identifier: String {
        "extra-key-\(String(describing: self))"
    }

    // a symbol key's character
    private var text: String {
        if case .type(let text) = action {
            return text
        }
        return ""
    }

    /// Its legend, the keyboard's key's as the soft keyboard is `keyboardOpen` or not.
    public func legend(keyboardOpen: Bool) -> ExtraKeyLegend {
        switch self {
        case .escape:
            .word(String(localized: "Esc"))

        case .tab:
            .word(String(localized: "Tab"))

        case .control:
            .word(String(localized: "Ctrl"))

        case .alt:
            .word(String(localized: "Alt"))

        case .slash, .pipe, .tilde:
            .symbol(text)

        case .home:
            .word(String(localized: "Home"))

        case .end:
            .word(String(localized: "End"))

        case .pageUp:
            .word(String(localized: "PgUp"))

        case .pageDown:
            .word(String(localized: "PgDn"))

        case .leftArrow:
            .image("arrow.left")

        case .upArrow:
            .image("arrow.up")

        case .downArrow:
            .image("arrow.down")

        case .rightArrow:
            .image("arrow.right")

        case .keyboard:
            .image(keyboardOpen ? "keyboard.chevron.compact.down" : "keyboard")
        }
    }

    /// How VoiceOver names it, the keyboard's key's as the soft keyboard is `keyboardOpen` or not.
    public func label(keyboardOpen: Bool) -> String {
        switch self {
        case .escape:
            String(localized: "Escape")

        case .tab:
            String(localized: "Tab")

        case .control:
            String(localized: "Control")

        case .alt:
            String(localized: "Alt")

        case .slash:
            String(localized: "Slash")

        case .pipe:
            String(localized: "Vertical bar")

        case .tilde:
            String(localized: "Tilde")

        case .home:
            String(localized: "Home")

        case .end:
            String(localized: "End")

        case .pageUp:
            String(localized: "Page up")

        case .pageDown:
            String(localized: "Page down")

        case .leftArrow:
            String(localized: "Left arrow")

        case .upArrow:
            String(localized: "Up arrow")

        case .downArrow:
            String(localized: "Down arrow")

        case .rightArrow:
            String(localized: "Right arrow")

        case .keyboard:
            if keyboardOpen {
                String(localized: "Hide keyboard")
            } else {
                String(localized: "Show keyboard")
            }
        }
    }
}
