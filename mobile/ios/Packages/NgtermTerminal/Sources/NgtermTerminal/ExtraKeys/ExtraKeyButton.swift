import NgtermDesign
import UIKit

/// One extra key: its whole cell takes the touch, and its face shows only what it holds (mobile/android's
/// `ExtraKeyButton`): nothing for a key at rest, the system's fill while pressed, an outline in the tint for a modifier
/// held for the next key, the tint filled for one locked.
internal final class ExtraKeyButton: UIControl {
    // the inset of the face in the cell, which takes the touch whole
    private static let inset: CGFloat = 2
    private static let edge: CGFloat = 1.5

    /// The key.
    internal let key: ExtraKey
    /// What VoiceOver's activation does: a press and its release.
    internal var activate: (@MainActor () -> Void)?

    private let face = UIView()
    private let word = UILabel()
    private let symbol = UIImageView()
    private var sticky = Sticky.off

    /// The look for where its modifier stands.
    override internal var isHighlighted: Bool {
        didSet {
            paint()
        }
    }

    /// The key `key`.
    internal init(_ key: ExtraKey) {
        self.key = key
        super.init(frame: .zero)
        face.isUserInteractionEnabled = false
        face.layer.cornerRadius = Radius.small
        face.layer.cornerCurve = .continuous
        addSubview(face)
        for legend in [word, symbol] as [UIView] {
            legend.isUserInteractionEnabled = false
            addSubview(legend)
        }
        word.textAlignment = .center
        word.adjustsFontForContentSizeCategory = false
        symbol.contentMode = .center
        isAccessibilityElement = true
        accessibilityTraits = .keyboardKey
        accessibilityIdentifier = key.identifier
        // the outline's color is resolved once: drawn again as the appearance changes
        let traits: [any UITraitDefinition.Type] = [UITraitUserInterfaceStyle.self, UITraitAccessibilityContrast.self]
        registerForTraitChanges(traits) { (button: Self, _: UITraitCollection) in
            button.paint()
        }
    }

    @available(*, unavailable, message: "an extra key is made in code")
    internal required init?(coder _: NSCoder) {
        nil
    }

    // what VoiceOver says of a modifier's state
    private static func words(for sticky: Sticky) -> String {
        switch sticky {
        case .off:
            String(localized: "Off")

        case .once:
            String(localized: "On for the next key")

        case .locked:
            String(localized: "Locked on")
        }
    }

    /// Shows the key for `sticky` (its modifier's state) and the soft keyboard `keyboardOpen`, its words in `wordFont`
    /// and its symbols in `symbolFont`.
    internal func show(sticky: Sticky, keyboardOpen: Bool, wordFont: UIFont, symbolFont: UIFont) {
        self.sticky = sticky
        word.isHidden = true
        symbol.isHidden = true
        switch key.legend(keyboardOpen: keyboardOpen) {
        case .word(let text):
            word.text = text
            word.font = wordFont
            word.isHidden = false

        case .symbol(let text):
            word.text = text
            word.font = symbolFont
            word.isHidden = false

        case .image(let name):
            let configuration = UIImage.SymbolConfiguration(font: wordFont, scale: .medium)
            symbol.image = UIImage(systemName: name, withConfiguration: configuration)
            symbol.isHidden = false
        }
        accessibilityLabel = key.label(keyboardOpen: keyboardOpen)
        if case .hold = key.action {
            accessibilityValue = Self.words(for: sticky)
        }
        paint()
    }

    /// Lays the face and the legend out in the cell.
    override internal func layoutSubviews() {
        super.layoutSubviews()
        let inner = bounds.insetBy(dx: Self.inset, dy: Self.inset)
        face.frame = inner
        word.frame = inner
        symbol.frame = inner
    }

    /// VoiceOver's activation: the key goes down and up.
    override internal func accessibilityActivate() -> Bool {
        activate?()
        return activate != nil
    }

    // the face, edge and legend for the state
    private func paint() {
        let tint = Palette.tint.uiColor
        let ink: UIColor
        switch sticky {
        case .off:
            face.backgroundColor = isHighlighted ? .systemFill : .clear
            face.layer.borderWidth = 0
            ink = .label

        case .once:
            face.backgroundColor = isHighlighted ? .systemFill : .clear
            face.layer.borderWidth = Self.edge
            face.layer.borderColor = tint.resolvedColor(with: traitCollection).cgColor
            ink = tint

        case .locked:
            face.backgroundColor = tint
            face.layer.borderWidth = 0
            ink = Palette.onTint.uiColor
        }
        word.textColor = ink
        symbol.tintColor = ink
    }

    deinit {
        // nothing to release
    }
}
