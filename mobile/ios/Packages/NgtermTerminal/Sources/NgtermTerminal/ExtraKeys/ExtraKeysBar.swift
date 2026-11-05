import NgtermDesign
import UIKit

/// The keys a soft keyboard lacks, over it: the terminal's `inputAccessoryView` (mobile/android's `ExtraKeysRow`).
///
/// Esc, Tab, a sticky Ctrl and Alt, `/`, `|` and `~`, Home and End, Page Up and Down, the arrows, and the soft
/// keyboard's own key, going to the ``TerminalKeyboard``'s target. Two lines of eight on a phone held upright, one
/// line of sixteen once the width gives each key 44 points. A key acts as it goes down, with the keyboard's click and a
/// light tap; the arrows and the page keys repeat while held. Ctrl and Alt hold for the next key (outlined in the
/// tint), lock on a second tap (filled), and let go on a third, with a selection tick. The keyboard's key closes the
/// soft keyboard and opens it again, the focus and the bar staying. It isn't shown while a hardware keyboard is
/// attached.
@preconcurrency
@MainActor
public final class ExtraKeysBar: UIInputView, UIInputViewAudioFeedback {
    /// A key's height, and the least width a key gets before the bar takes one line.
    public static let keyHeight = CGFloat(Spacing.hitTarget)

    // the share of a key's room the widest word takes, leaving its glyphs' overhang room
    private static let fit: CGFloat = 0.9
    // the room a word keeps from its key's edges, besides the face's inset
    private static let legendInset: CGFloat = 6
    // a symbol is one character: a size up from the words, as the terminal draws it
    private static let symbolSize: CGFloat = 18
    private static let largestSymbol: CGFloat = 26
    // a key's two sides
    private static let sides: CGFloat = 2

    // the keyboard owns the bar, but UIKit may keep it a moment longer, while it goes off screen
    private weak var keyboard: TerminalKeyboard?
    private let content = UIView()
    private let repeats = KeyRepeat()
    // the key held whose repeat runs
    private var repeatingKey: ExtraKey?
    private var buttons: [ExtraKey: ExtraKeyButton] = [:]
    private var height: NSLayoutConstraint?

    /// The lines of keys shown now.
    public private(set) var rows = ExtraKey.twoRows

    /// The keyboard's click sounds as the user set them.
    public var enableInputClicksWhenVisible: Bool {
        true
    }

    /// The bar of `keyboard`.
    internal init(keyboard: TerminalKeyboard) {
        self.keyboard = keyboard
        let size = CGSize(width: 0, height: Self.keyHeight * CGFloat(ExtraKey.twoRows.count))
        super.init(frame: CGRect(origin: .zero, size: size), inputViewStyle: .keyboard)
        allowsSelfSizing = true
        accessibilityLabel = String(localized: "Extra keys")
        accessibilityIdentifier = "extra-keys"
        content.translatesAutoresizingMaskIntoConstraints = false
        addSubview(content)
        let keysHeight = content.heightAnchor.constraint(equalToConstant: size.height)
        // the keys' height decides the bar's: its foot gives way while the bar takes a new height
        let foot = content.bottomAnchor.constraint(equalTo: safeAreaLayoutGuide.bottomAnchor)
        foot.priority = .required - 1
        NSLayoutConstraint.activate([
            content.topAnchor.constraint(equalTo: topAnchor),
            content.leadingAnchor.constraint(equalTo: safeAreaLayoutGuide.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: safeAreaLayoutGuide.trailingAnchor),
            foot,
            keysHeight,
        ])
        height = keysHeight
        for key in ExtraKey.allCases {
            let button = ExtraKeyButton(key)
            button.addTarget(self, action: #selector(down), for: .touchDown)
            button.addTarget(self, action: #selector(up), for: [.touchUpInside, .touchUpOutside, .touchCancel])
            button.activate = { [weak self] in
                self?.press(key)
                self?.release(key)
            }
            content.addSubview(button)
            buttons[key] = button
        }
        registerForTraitChanges([UITraitPreferredContentSizeCategory.self]) { (bar: Self, _: UITraitCollection) in
            bar.setNeedsLayout()
        }
        refresh()
    }

    @available(*, unavailable, message: "the extra keys are made in code")
    internal required init?(coder _: NSCoder) {
        nil
    }

    /// The lines of keys for `width` points: one line once every key gets ``keyHeight``, else two.
    public static func rows(forWidth width: CGFloat) -> [[ExtraKey]] {
        width >= keyHeight * CGFloat(ExtraKey.oneRow.count) ? [ExtraKey.oneRow] : ExtraKey.twoRows
    }

    /// A key goes down: it acts, with the keyboard's click and a tap, and repeats while held if it does.
    public func press(_ key: ExtraKey) {
        guard let keyboard else {
            return
        }
        UIDevice.current.playInputClick()
        switch key.action {
        case .hold(let modifier):
            keyboard.toggle(modifier)
            UISelectionFeedbackGenerator(view: self).selectionChanged()

        case .press(let name, let repeating):
            keyboard.press(name)
            tap()
            if repeating {
                repeatingKey = key
                repeats.start { [weak self] in
                    self?.keyboard?.press(name)
                }
            }

        case .type(let text):
            keyboard.type(text)
            tap()

        case .softKeyboard:
            tap()
            if let target = keyboard.target {
                if target.softKeyboardWanted {
                    target.hideSoftKeyboard()
                } else {
                    target.focus(showingKeyboard: true)
                }
            }
        }
    }

    /// `key` goes up: its repeat stops, if it is the key repeating; a key pressed since repeats on.
    public func release(_ key: ExtraKey) {
        if repeatingKey == key {
            repeats.stop()
            repeatingKey = nil
        }
    }

    /// Shows each key as things stand: the sticky modifiers, and whether the soft keyboard is open.
    public func refresh() {
        guard let keyboard else {
            return
        }
        let open = keyboard.target?.softKeyboardWanted ?? false
        let fonts = legendFonts()
        for (key, button) in buttons {
            let sticky = if case .hold(let modifier) = key.action { keyboard.sticky(modifier) } else { Sticky.off }
            button.show(sticky: sticky, keyboardOpen: open, wordFont: fonts.word, symbolFont: fonts.symbol)
        }
    }

    /// Lays the keys out in their lines, each its whole cell with no gaps, taking one line once the width allows.
    override public func layoutSubviews() {
        super.layoutSubviews()
        let fitting = Self.rows(forWidth: content.bounds.width)
        if fitting != rows {
            rows = fitting
            height?.constant = Self.keyHeight * CGFloat(rows.count)
        }
        refresh()
        let lineHeight = content.bounds.height / CGFloat(max(rows.count, 1))
        for (line, keys) in rows.enumerated() {
            let width = content.bounds.width / CGFloat(max(keys.count, 1))
            for (column, key) in keys.enumerated() {
                buttons[key]?.frame = CGRect(
                    x: CGFloat(column) * width,
                    y: CGFloat(line) * lineHeight,
                    width: width,
                    height: lineHeight
                )
            }
        }
        let shown = Set(rows.joined())
        for (key, button) in buttons {
            button.isHidden = !shown.contains(key)
        }
    }

    // the words at one size, the widest's ("PgUp") shrunk only as far as its key needs; the symbols in the
    // terminal's face, a size up
    private func legendFonts() -> (word: UIFont, symbol: UIFont) {
        let base = UIFont.preferredFont(forTextStyle: .subheadline, compatibleWith: traitCollection)
        let word = UIFont.systemFont(ofSize: base.pointSize, weight: .semibold)
        let keys = rows.first?.count ?? 1
        let room = content.bounds.width / CGFloat(keys) - Self.sides * Self.legendInset
        let widths = ExtraKey.allCases.compactMap { key -> CGFloat? in
            guard case .word(let text) = key.legend(keyboardOpen: false) else {
                return nil
            }
            return NSAttributedString(string: text, attributes: [.font: word]).size().width
        }
        let widest = widths.max() ?? 1
        let scale = room > 0 ? min(1, room * Self.fit / widest) : 1
        let metrics = UIFontMetrics(forTextStyle: .body)
        let scaled = metrics.scaledValue(for: Self.symbolSize, compatibleWith: traitCollection)
        let size = min(scaled, Self.largestSymbol)
        let symbol =
            UIFont(name: Fonts.terminalFaces.regular, size: size)
            ?? .monospacedSystemFont(ofSize: size, weight: .regular)
        return (word.withSize(word.pointSize * scale), symbol)
    }

    // a key's tap, as a keyboard's
    private func tap() {
        UIImpactFeedbackGenerator(style: .light, view: self).impactOccurred()
    }

    @objc
    private func down(_ button: ExtraKeyButton) {
        press(button.key)
    }

    @objc
    private func up(_ button: ExtraKeyButton) {
        release(button.key)
    }

    deinit {
        // the repeat stops as it goes
    }
}
