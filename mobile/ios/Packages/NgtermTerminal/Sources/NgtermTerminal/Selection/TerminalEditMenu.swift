import UIKit

/// The terminal's edit menu (`UIEditMenuInteraction`): Copy while there is a selection, Paste while the clipboard
/// holds text, and Select All, over a touch selection or where the pointer's secondary button clicked.
///
/// Its items are the standard edit actions (`copy(_:)`, `paste(_:)`, `selectAll(_:)`), which the view answers as the
/// first responder, so a paste from the menu is the user's own and iOS doesn't ask to allow it.
@MainActor
internal final class TerminalEditMenu: NSObject, @preconcurrency UIEditMenuInteractionDelegate {
    private unowned let view: TerminalView
    private lazy var interaction = UIEditMenuInteraction(delegate: self)
    // what the menu points at: the selection's rows, or a point
    private var target: CGRect = .null

    /// Whether the menu shows.
    internal private(set) var isShown = false

    internal init(view: TerminalView) {
        self.view = view
        super.init()
        view.addInteraction(interaction)
    }

    /// Shows the menu over `rows` (a touch selection's), or under them where there is no room above.
    internal func show(over rows: CGRect) {
        target = rows
        present(at: CGPoint(x: rows.midX, y: rows.minY))
    }

    /// Shows the menu at `point` (a secondary click).
    internal func show(at point: CGPoint) {
        target = .null
        present(at: point)
    }

    /// Takes the menu down.
    internal func hide() {
        guard isShown else {
            return
        }
        interaction.dismissMenu()
        isShown = false
    }

    /// Takes the menu down when it shows over a touch selection's rows: the one at a secondary click's point stays
    /// until the user picks an item or taps elsewhere.
    internal func hideOverSelection() {
        if !target.isNull {
            hide()
        }
    }

    // MARK: - UIEditMenuInteractionDelegate

    /// The menu's items: the standard edit actions the view can perform now.
    internal func editMenuInteraction(
        _: UIEditMenuInteraction,
        menuFor _: UIEditMenuConfiguration,
        suggestedActions _: [UIMenuElement]
    ) -> UIMenu? {
        let commands = TerminalView.editActions.compactMap { action, title in
            view.canPerformAction(action, withSender: nil) ? UICommand(title: title, action: action) : nil
        }
        return UIMenu(children: commands)
    }

    /// Points the menu at the selection's rows when it is a touch selection's.
    internal func editMenuInteraction(
        _: UIEditMenuInteraction,
        targetRectFor _: UIEditMenuConfiguration
    ) -> CGRect {
        target
    }

    /// The menu went.
    internal func editMenuInteraction(
        _: UIEditMenuInteraction,
        willDismissMenuFor _: UIEditMenuConfiguration,
        animator _: any UIEditMenuInteractionAnimating
    ) {
        isShown = false
    }

    private func present(at point: CGPoint) {
        _ = view.becomeFirstResponder()
        interaction.presentEditMenu(with: UIEditMenuConfiguration(identifier: nil, sourcePoint: point))
        isShown = true
    }

    deinit {
        // the view holds the interaction
    }
}
