/// The keyboard's side of the terminals on screen (mobile/android's `TerminalKeyboard`).
///
/// What the soft keyboard types, the extra keys and a hardware keyboard's keys go to the ``target`` and nowhere else,
/// and only while it is drawn focused (the selected pane), with the extra keys' sticky Ctrl and Alt held for the next
/// key. One keyboard serves every terminal on screen, so
/// the sticky modifiers stay as the focus moves. The target is the terminal that is the first responder. A key from
/// elsewhere than the soft keyboard (an extra key, a sticky tap) commits the composition waiting in the target first,
/// so the program gets the text before the key, in the order it was typed.
@preconcurrency
@MainActor
public final class TerminalKeyboard {
    private static let enter = "enter"

    private var madeBar: ExtraKeysBar?
    // the focus the terminal that had it left when the selection moved on: the terminal selected next takes it
    private var handingOver: Handover?

    /// The moves' actions, which the session view sets.
    public var shortcuts: TerminalShortcuts
    /// Whether a hardware keyboard is attached.
    public let hardware: HardwareKeyboard
    /// The sticky Ctrl.
    public private(set) var control = Sticky.off
    /// The sticky Alt.
    public private(set) var alt = Sticky.off
    /// The terminal every key goes to: the one that became the first responder last, if it still is.
    public weak var target: TerminalView?
    /// Which chat's prompt field has the keyboard, if one does: the session view's chats say so, as a terminal says
    /// it is the ``target``, so a move of the selection hands the keyboard on from a chat too.
    public var chatField: AnyHashable?

    /// The extra keys, over the soft keyboard.
    public var bar: ExtraKeysBar {
        if let madeBar {
            return madeBar
        }
        let made = ExtraKeysBar(keyboard: self)
        madeBar = made
        return made
    }

    /// How the extra keys lay out, as Settings sets it: the terminal with the focus shows a change at once.
    ///
    /// Hiding them lets go of the sticky Ctrl and Alt, whose keys go with them.
    public var layout = ExtraKeysLayout.fit {
        didSet {
            guard layout != oldValue else {
                return
            }
            if layout == .hidden {
                set(.control, .off)
                set(.alt, .off)
            }
            madeBar?.setNeedsLayout()
            target?.reloadInputViews()
        }
    }

    /// Whether the extra keys show: not while a hardware keyboard is attached, which has every key they have, nor
    /// when Settings hides them.
    public var showsExtraKeys: Bool {
        !hardware.isAttached && layout != .hidden
    }

    /// A keyboard running `shortcuts`, the extra keys standing aside while `hardware` says a keyboard is attached (the
    /// system's by default).
    public init(shortcuts: TerminalShortcuts = TerminalShortcuts(), hardware: HardwareKeyboard? = nil) {
        self.shortcuts = shortcuts
        self.hardware = hardware ?? HardwareKeyboard()
        self.hardware.onChange = { [weak self] in
            self?.target?.reloadInputViews()
        }
    }

    /// Where `modifier` stands.
    public func sticky(_ modifier: StickyModifier) -> Sticky {
        switch modifier {
        case .control:
            control

        case .alt:
            alt
        }
    }

    /// A tap on the key of `modifier`, returning where it stands now: off to once, once to locked, locked to off.
    ///
    /// What the target composes was typed before the tap, so it goes first, under the modifiers as they stood.
    @discardableResult
    public func toggle(_ modifier: StickyModifier) -> Sticky {
        target?.commitComposition()
        let next = sticky(modifier).next
        set(modifier, next)
        return next
    }

    /// An extra key's text for the target, after its composition: as ``type(_:to:)``.
    public func type(_ text: String) {
        guard let target else {
            return
        }
        target.commitComposition()
        type(text, to: target)
    }

    /// Presses and lets go of the key Ghostty names `name` on the target, with the sticky modifiers, after its
    /// composition (an extra key).
    public func press(_ name: String) {
        guard let target else {
            return
        }
        target.commitComposition()
        press(name, to: target)
    }

    /// Text for `view`: line breaks go as Enter, the rest as typed text; with a sticky modifier held, the first
    /// character goes as the key that types it, with the modifier.
    ///
    /// A view not drawn focused takes nothing.
    public func type(_ text: String, to view: TerminalView) {
        guard view.drawsFocused else {
            return
        }
        var rest = Substring(text)
        if control != .off || alt != .off, let first = rest.first, first != "\n" {
            view.pressKey(KeyMapping.forText(String(first), mods: takeSticky()))
            rest = rest.dropFirst()
        }
        for (index, line) in rest.split(separator: "\n", omittingEmptySubsequences: false).enumerated() {
            if index > 0 {
                press(Self.enter, to: view)
            }
            if !line.isEmpty {
                view.send(text: String(line))
            }
        }
    }

    /// Presses and lets go of the key Ghostty names `name` on `view`, with the sticky modifiers; a view not drawn
    /// focused takes nothing.
    public func press(_ name: String, to view: TerminalView) {
        guard view.drawsFocused else {
            return
        }
        view.pressKey(KeyMapping.named(name, mods: takeSticky()))
    }

    /// The sticky modifiers' bits for a key pressed now; a modifier armed once lets go with it.
    public func takeSticky() -> UInt8 {
        var bits: UInt8 = 0
        for modifier in StickyModifier.allCases {
            let state = sticky(modifier)
            if state != .off {
                bits |= modifier.bit
            }
            if state == .once {
                set(modifier, .off)
            }
        }
        return bits
    }

    /// The session view moves the selection to another pane, or builds the selected one anew (another layout).
    ///
    /// When a terminal has the focus now, the one drawn focused next in a window takes it, with the soft keyboard if
    /// this one had it open (Android's `FollowFocus`). A chat's prompt field with the keyboard (``chatField``) hands it
    /// on the same way, with the soft keyboard.
    public func handOver() {
        if let target {
            handingOver = target.softKeyboardWanted ? .withKeyboard : .focus
        } else {
            handingOver = chatField == nil ? nil : .withKeyboard
        }
    }

    /// Whether ``handOver()`` left the keyboard for the chat selected next, which takes its prompt field's focus;
    /// taken once.
    public func takeChatHandover() -> Bool {
        takeHandover() != nil
    }

    /// What ``handOver()`` left for the terminal selected next, taken once.
    internal func takeHandover() -> Handover? {
        defer { handingOver = nil }
        return handingOver
    }

    /// Shows the extra keys as things stand now: the sticky modifiers, and whether the soft keyboard is open.
    internal func refreshBar() {
        madeBar?.refresh()
    }

    private func set(_ modifier: StickyModifier, _ state: Sticky) {
        switch modifier {
        case .control:
            control = state

        case .alt:
            alt = state
        }
        madeBar?.refresh()
    }

    deinit {
        // the hardware keyboard's change runs through a weak reference
    }
}
