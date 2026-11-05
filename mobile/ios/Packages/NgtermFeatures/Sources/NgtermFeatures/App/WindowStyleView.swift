import UIKit

/// An empty view that sets its window's light or dark style: the window's whole tree follows it, the sheets over it
/// included, and the system's style again once it is `.unspecified`.
internal final class WindowStyleView: UIView {
    /// The style its window takes.
    internal var style = UIUserInterfaceStyle.unspecified {
        didSet {
            apply()
        }
    }

    override internal init(frame: CGRect) {
        super.init(frame: frame)
        isUserInteractionEnabled = false
        isAccessibilityElement = false
        accessibilityElementsHidden = true
    }

    @available(*, unavailable, message: "made in code")
    internal required init?(coder _: NSCoder) {
        nil
    }

    /// It sets the style of the window it joins.
    override internal func didMoveToWindow() {
        super.didMoveToWindow()
        apply()
    }

    private func apply() {
        if let window, window.overrideUserInterfaceStyle != style {
            window.overrideUserInterfaceStyle = style
        }
    }

    deinit {
        // the window keeps the style it was given
    }
}
