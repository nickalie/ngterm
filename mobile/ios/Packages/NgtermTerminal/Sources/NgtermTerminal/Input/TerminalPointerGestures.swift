import NgtermCore
import UIKit

/// A mouse's or trackpad's input on a ``TerminalView`` on an iPad, as mobile/android's `terminalMouse` and
/// `mouseGesture` have it.
///
/// - a button pressed gives the terminal the focus; under mouse tracking (shift held overrides it) the press, the drag
///   and the release go to the program; otherwise a click lets the selection go, a double click selects the word and a
///   triple one the line, a drag selects, and the secondary button opens the edit menu;
/// - the scroll wheel, or a trackpad's two-finger scroll, scrolls the pane, which reports it to a program that asked
///   for the mouse;
/// - the pointer moving with no button down reports its motion to a program tracking it;
/// - the pointer is a text beam over the terminal, the system's arrow while a program takes the mouse.
///
/// Each handler is what its recognizer calls, and can be called on its own.
@preconcurrency
@MainActor
public final class TerminalPointerGestures: NSObject, UIPointerInteractionDelegate {
    // what a held button does: reports to the program (with the cell last reported), selects, or nothing
    private enum Press {
        case idle
        case report(PaneMouseButton, CellPoint?)
        case select(SelectionGrip)
    }

    // how near a second click lands to the first to count as a double click, in points
    private static let clickReach: CGFloat = 4
    private static let middleButton = 3

    private unowned let view: TerminalView
    private var clicks = ClickCounter()
    private var press = Press.idle
    private var scrolled: CGPoint = .zero

    internal init(view: TerminalView) {
        self.view = view
        super.init()
        install()
    }

    /// A button press at `point` with `buttons` down and `mods` held, at `time` (seconds of the system's uptime), and
    /// the drag and release that follow it.
    public func press(
        _ phase: GesturePhase,
        at point: CGPoint,
        buttons: UIEvent.ButtonMask,
        mods: UIKeyModifierFlags,
        time: TimeInterval
    ) {
        switch phase {
        case .began:
            pressed(at: point, buttons: buttons, mods: mods, time: time)

        case .changed:
            moved(to: point, mods: mods)

        case .ended, .cancelled:
            if case .report(let button, _) = press {
                view.pointer.mouse(.release, button: button, mods: mods.paneBits, at: point)
            }
            press = .idle
        }
    }

    /// The scroll wheel or a two-finger scroll, `translation` points from where it started (as the fingers go).
    public func scroll(_ phase: GesturePhase, translation: CGPoint, at point: CGPoint, mods: UIKeyModifierFlags) {
        if phase == .began {
            view.pointer.stopFling()
            scrolled = .zero
        }
        guard phase != .cancelled else {
            return
        }
        let delta = CGPoint(x: translation.x - scrolled.x, y: translation.y - scrolled.y)
        scrolled = translation
        view.pointer.scroll(by: delta, mods: mods.paneBits, at: point)
    }

    /// The pointer over `point` with no button down and `mods` held.
    public func hover(at point: CGPoint, mods: UIKeyModifierFlags) {
        if view.pointer.tracking {
            view.pointer.hover(at: point, mods: mods.paneBits)
        }
    }

    // MARK: - UIPointerInteractionDelegate

    /// A text beam a row tall over the terminal; the system's pointer while a program takes the mouse.
    public func pointerInteraction(_: UIPointerInteraction, styleFor _: UIPointerRegion) -> UIPointerStyle? {
        view.pointer.tracking ? nil : UIPointerStyle(shape: .verticalBeam(length: view.font.cellHeight))
    }

    private func pressed(at point: CGPoint, buttons: UIEvent.ButtonMask, mods: UIKeyModifierFlags, time: TimeInterval) {
        view.focus()
        let pointer = view.pointer
        let button: PaneMouseButton =
            if buttons.contains(.secondary) {
                .right
            } else if buttons.contains(.button(Self.middleButton)) {
                .middle
            } else {
                .left
            }
        if pointer.tracking, !mods.contains(.shift) {
            pointer.mouse(.press, button: button, mods: mods.paneBits, at: point)
            press = .report(button, pointer.cell(at: point))
            return
        }
        press = .idle
        guard let cell = pointer.cell(at: point) else {
            return
        }
        switch button {
        case .right:
            view.showMenu(at: point)

        case .left:
            let count = clicks.click(at: point, time: time, reach: Self.clickReach)
            view.selection.click(count, at: cell)
            if count == 1 {
                press = .select(.plain(fixed: cell, moving: cell, offset: .zero))
            }

        case .middle:
            break
        }
    }

    private func moved(to point: CGPoint, mods: UIKeyModifierFlags) {
        let pointer = view.pointer
        switch press {
        case .report(let button, let last):
            let cell = pointer.cell(at: point)
            if cell != last {
                pointer.mouse(.motion, button: button, mods: mods.paneBits, at: point)
                press = .report(button, cell)
            }

        case .select(var grip):
            if let cell = pointer.cell(at: point) {
                grip.move(to: cell, in: view.selection, spans: view.screen.selection)
                press = .select(grip)
            }

        case .idle:
            break
        }
    }

    private func install() {
        let pressing = PointerPressRecognizer(target: self, action: #selector(pressedRecognizer))
        let scrolling = UIPanGestureRecognizer(target: self, action: #selector(scrolledRecognizer))
        scrolling.allowedScrollTypesMask = .all
        scrolling.allowedTouchTypes = []
        let hovering = UIHoverGestureRecognizer(target: self, action: #selector(hovered))
        for recognizer in [pressing, scrolling, hovering] {
            view.addGestureRecognizer(recognizer)
        }
        view.addInteraction(UIPointerInteraction(delegate: self))
    }

    @objc
    private func pressedRecognizer(_ recognizer: PointerPressRecognizer) {
        if let phase = GesturePhase(recognizer.state) {
            press(
                phase,
                at: recognizer.location(in: view),
                buttons: recognizer.buttons,
                mods: recognizer.modifiers,
                time: recognizer.pressedAt
            )
        }
    }

    @objc
    private func scrolledRecognizer(_ recognizer: UIPanGestureRecognizer) {
        if let phase = GesturePhase(recognizer.state) {
            scroll(
                phase,
                translation: recognizer.translation(in: view),
                at: recognizer.location(in: view),
                mods: recognizer.modifierFlags
            )
        }
    }

    @objc
    private func hovered(_ recognizer: UIHoverGestureRecognizer) {
        if recognizer.state == .began || recognizer.state == .changed {
            hover(at: recognizer.location(in: view), mods: recognizer.modifierFlags)
        }
    }

    deinit {
        // the view holds the recognizers and the interaction, which hold their targets and delegate weakly
    }
}
