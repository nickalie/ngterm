import NgtermTerminal
import Testing
import UIKit

/// What a recognizer's state says to the terminal's gesture handlers.
@Suite("a gesture's phase")
internal struct GesturePhaseTests {
    @Test(
        "a recognizer's moving states are the phases of the same names",
        arguments: [
            (UIGestureRecognizer.State.began, GesturePhase.began),
            (.changed, .changed),
            (.ended, .ended),
            (.cancelled, .cancelled),
        ]
    )
    internal func phases(state: UIGestureRecognizer.State, phase: GesturePhase) {
        #expect(GesturePhase(state) == phase)
    }

    @Test(
        "a recognizer not yet started, or failed, is in no phase",
        arguments: [UIGestureRecognizer.State.possible, .failed]
    )
    internal func none(state: UIGestureRecognizer.State) {
        #expect(GesturePhase(state) == nil)
    }
}
