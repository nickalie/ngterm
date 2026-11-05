import UIKit

/// Where a continuous gesture is: what a recognizer's state says to the terminal's handlers.
public enum GesturePhase: Sendable {
    /// It started.
    case began
    /// The system took it back (a call came in, another gesture won): it does nothing more.
    case cancelled
    /// It moved.
    case changed
    /// It ended as the user meant it to.
    case ended

    /// The phase of a recognizer in `state`; `nil` for a state that isn't one (possible, failed).
    public init?(_ state: UIGestureRecognizer.State) {
        switch state {
        case .began:
            self = .began

        case .changed:
            self = .changed

        case .ended:
            self = .ended

        case .cancelled:
            self = .cancelled

        default:
            return nil
        }
    }
}
