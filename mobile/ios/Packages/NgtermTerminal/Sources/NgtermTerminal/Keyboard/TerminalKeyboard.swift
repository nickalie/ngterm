/// The keyboard's side of the terminals on screen (mobile/android's `TerminalKeyboard`).
///
/// What the soft keyboard types, the extra keys and a hardware keyboard's keys go to the ``target`` and nowhere else,
/// with the extra keys' sticky Ctrl and Alt held for the next key. One keyboard serves every terminal on screen, so
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

    /// The extra keys, over the soft keyboard.
    public var bar: ExtraKeysBar {
        if let madeBar {
            return madeBar
        }
        let made = ExtraKeysBar(keyboard: self)
        madeBar = made
        return made
    }

    /// Whether the extra keys show: not while a hardware keyboard is attached, which has every key they have.
    public var showsExtraKeys: Bool {
        !hardware.isAttached
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
    public func type(_ text: String, to view: TerminalView) {
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

    /// Presses and lets go of the key Ghostty names `name` on `view`, with the sticky modifiers.
    public func press(_ name: String, to view: TerminalView) {
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

    /// The session view moves the selection to another terminal: when a terminal has the focus now, the one drawn
    /// focused next in a window takes it, with the soft keyboard if this one had it open (Android's `FollowFocus`).
    public func handOver() {
        handingOver = target.map { $0.softKeyboardWanted ? .withKeyboard : .focus }
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
