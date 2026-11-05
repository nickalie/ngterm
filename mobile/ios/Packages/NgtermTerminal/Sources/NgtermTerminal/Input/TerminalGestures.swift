import UIKit

/// A finger's gestures on a ``TerminalView``, as mobile/android's `terminalGestures` has them.
///
/// - a tap gives the terminal the focus, and under mouse tracking clicks there; a tap lets a touch selection go;
/// - a double tap selects the word, a triple tap the line, with handles and the edit menu;
/// - a drag scrolls (the pane reports it to a program that asked for the mouse, as wheel turns) and flings on; a drag
///   from a selection handle moves that end;
/// - a long press selects the word with handles, and a drag from it extends the selection, as text selection does; the
///   edit menu shows when the finger lifts;
/// - a pinch changes the text size, in whole points.
///
/// A touch the system cancels does none of these, and a cancelled drag doesn't fling. Each handler is what its
/// recognizer calls, and can be called on its own.
@preconcurrency
@MainActor
public final class TerminalGestures: NSObject, UIGestureRecognizerDelegate {
    // how near a second tap lands to the first to count as a double tap, in points
    private static let tapReach: CGFloat = 22
    // a single, a double (word) and a triple (line) tap, then round again
    private static let tapKinds = 3

    private unowned let view: TerminalView
    private var taps = ClickCounter()
    // what a drag holds: a handle's grip while it moves an end, none while it scrolls
    private var dragGrip: SelectionGrip?
    private var dragTranslation: CGPoint = .zero
    private var pressGrip: SelectionGrip?
    private var pinchBase: CGFloat?

    internal init(view: TerminalView) {
        self.view = view
        super.init()
        install()
    }

    /// A tap at `point` at `time` (seconds of the system's uptime).
    public func tap(at point: CGPoint, time: TimeInterval) {
        let count = taps.click(at: point, time: time, reach: Self.tapReach)
        let pointer = view.pointer
        let selection = view.selection
        if pointer.tracking {
            pointer.mouse(.press, button: .left, mods: 0, at: point)
            pointer.mouse(.release, button: .left, mods: 0, at: point)
            view.focus()
            return
        }
        switch (count - 1) % Self.tapKinds {
        case 0:
            if selection.isTouchSelection {
                selection.clear()
            } else {
                view.focus()
            }

        case 1:
            if let cell = pointer.cell(at: point) {
                selection.selectWord(at: cell)
                showMenu()
            }

        default:
            if let cell = pointer.cell(at: point) {
                selection.selectLine(at: cell)
                showMenu()
            }
        }
    }

    /// A long press at `point`: it selects the word there as it begins, and the finger's drag extends the selection.
    public func longPress(_ phase: GesturePhase, at point: CGPoint) {
        let selection = view.selection
        switch phase {
        case .began:
            view.pointer.stopFling()
            guard let cell = view.pointer.cell(at: point) else {
                return
            }
            UISelectionFeedbackGenerator(view: view).selectionChanged(at: point)
            view.hideMenu()
            selection.selectWord(at: cell)
            selection.isAdjusting = true
            pressGrip = .word(at: cell)

        case .changed:
            if let cell = view.pointer.cell(at: point) {
                pressGrip?.move(to: cell, in: selection, spans: view.screen.selection)
            }

        case .ended, .cancelled:
            guard pressGrip != nil else {
                return
            }
            pressGrip = nil
            selection.isAdjusting = false
            if phase == .ended {
                showMenu()
            }
        }
    }

    /// A one-finger drag, now at `point`, `translation` from where it started, moving at `velocity` points a second.
    public func pan(_ phase: GesturePhase, at point: CGPoint, translation: CGPoint, velocity: CGPoint) {
        switch phase {
        case .began:
            dragTranslation = .zero
            view.pointer.stopFling()
            let start = CGPoint(x: point.x - translation.x, y: point.y - translation.y)
            if let handle = view.handleSpots?.nearest(to: start) {
                let offset = CGPoint(x: handle.cellCenter.x - start.x, y: handle.cellCenter.y - start.y)
                dragGrip = .plain(fixed: handle.fixed, moving: handle.cell, offset: offset)
                view.selection.isAdjusting = true
                view.hideMenu()
            }
            dragged(to: point, translation: translation)

        case .changed:
            dragged(to: point, translation: translation)

        case .ended, .cancelled:
            if dragGrip != nil {
                dragGrip = nil
                view.selection.isAdjusting = false
                if phase == .ended {
                    showMenu()
                }
            } else if phase == .ended, pinchBase == nil {
                view.pointer.fling(velocity: velocity.y, at: point)
            }
        }
    }

    /// A pinch, now at `scale` times the text size it started at.
    public func pinch(_ phase: GesturePhase, scale: CGFloat) {
        switch phase {
        case .began:
            view.pointer.stopFling()
            pinchBase = view.textSize

        case .changed:
            if let pinchBase {
                view.pointer.zoom(base: pinchBase, scale: scale)
            }

        case .ended, .cancelled:
            pinchBase = nil
        }
    }

    // MARK: - UIGestureRecognizerDelegate

    /// A long press doesn't start on a selection handle: a finger held there takes the handle, not a new word.
    public func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
        guard recognizer is UILongPressGestureRecognizer else {
            return true
        }
        return view.handleSpots?.nearest(to: recognizer.location(in: view)) == nil
    }

    /// A pinch and a drag go together: while the pinch runs, the drag scrolls nothing.
    public func gestureRecognizer(
        _ first: UIGestureRecognizer,
        shouldRecognizeSimultaneouslyWith second: UIGestureRecognizer
    ) -> Bool {
        let pair = [first, second]
        return pair.contains { $0 is UIPinchGestureRecognizer } && pair.contains { $0 is UIPanGestureRecognizer }
    }

    private func dragged(to point: CGPoint, translation: CGPoint) {
        if var grip = dragGrip {
            let moved = CGPoint(x: point.x + grip.offset.x, y: point.y + grip.offset.y)
            if let cell = view.pointer.cell(at: moved) {
                grip.move(to: cell, in: view.selection, spans: view.screen.selection)
                dragGrip = grip
            }
        } else if pinchBase == nil {
            view.pointer.drag(by: translation.y - dragTranslation.y, at: point)
        }
        dragTranslation = translation
    }

    // the edit menu shows over the touch selection once a frame shows it
    private func showMenu() {
        view.selection.wantsMenu = view.selection.isTouchSelection
        view.refreshHandles()
    }

    private func install() {
        let tap = UITapGestureRecognizer(target: self, action: #selector(tapped))
        let press = UILongPressGestureRecognizer(target: self, action: #selector(pressed))
        let pan = UIPanGestureRecognizer(target: self, action: #selector(panned))
        pan.maximumNumberOfTouches = 1
        let pinch = UIPinchGestureRecognizer(target: self, action: #selector(pinched))
        for recognizer in [tap, press, pan] {
            recognizer.allowedTouchTypes = [.init(value: UITouch.TouchType.direct.rawValue)]
        }
        for recognizer in [tap, press, pan, pinch] {
            recognizer.delegate = self
            view.addGestureRecognizer(recognizer)
        }
    }

    @objc
    private func tapped(_ recognizer: UITapGestureRecognizer) {
        if recognizer.state == .ended {
            tap(at: recognizer.location(in: view), time: ProcessInfo.processInfo.systemUptime)
        }
    }

    @objc
    private func pressed(_ recognizer: UILongPressGestureRecognizer) {
        if let phase = GesturePhase(recognizer.state) {
            longPress(phase, at: recognizer.location(in: view))
        }
    }

    @objc
    private func panned(_ recognizer: UIPanGestureRecognizer) {
        if let phase = GesturePhase(recognizer.state) {
            pan(
                phase,
                at: recognizer.location(in: view),
                translation: recognizer.translation(in: view),
                velocity: recognizer.velocity(in: view)
            )
        }
    }

    @objc
    private func pinched(_ recognizer: UIPinchGestureRecognizer) {
        if let phase = GesturePhase(recognizer.state) {
            pinch(phase, scale: recognizer.scale)
        }
    }

    deinit {
        // the view holds the recognizers, which hold their targets weakly
    }
}
