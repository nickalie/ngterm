import UIKit.UIGestureRecognizerSubclass

/// A mouse's or trackpad's button held on the view, from its press to its release.
///
/// It begins at once on the press (no movement needed, unlike a pan), changes as the pointer moves, and ends on the
/// release, with the buttons pressed and the modifiers held. It takes only the pointer's touches, never a finger's.
internal final class PointerPressRecognizer: UIGestureRecognizer {
    /// The buttons pressed at the press.
    internal private(set) var buttons: UIEvent.ButtonMask = []
    /// The modifiers held now.
    internal private(set) var modifiers: UIKeyModifierFlags = []
    /// When the press came, in seconds of the system's uptime.
    internal private(set) var pressedAt: TimeInterval = 0

    /// A recognizer of the pointer's presses, reporting to `action` on `target`.
    override internal init(target: Any?, action: Selector?) {
        super.init(target: target, action: action)
        allowedTouchTypes = [.init(value: UITouch.TouchType.indirectPointer.rawValue)]
        cancelsTouchesInView = false
    }

    /// The press: it begins.
    override internal func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesBegan(touches, with: event)
        buttons = event.buttonMask
        modifiers = event.modifierFlags
        pressedAt = touches.first?.timestamp ?? event.timestamp
        state = .began
    }

    /// The pointer moved with the button held.
    override internal func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesMoved(touches, with: event)
        modifiers = event.modifierFlags
        state = .changed
    }

    /// The release.
    override internal func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesEnded(touches, with: event)
        modifiers = event.modifierFlags
        state = .ended
    }

    /// The system took the press back.
    override internal func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesCancelled(touches, with: event)
        state = .cancelled
    }

    /// Ready for the next press.
    override internal func reset() {
        super.reset()
        buttons = []
    }

    deinit {
        // nothing to release
    }
}
