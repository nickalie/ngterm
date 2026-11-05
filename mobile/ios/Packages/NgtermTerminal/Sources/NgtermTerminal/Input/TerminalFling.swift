import QuartzCore

/// A fling in progress: on each screen refresh it hands its step the points it went since the last one
/// (``FlingDecay``), until it slows to a stop, its step says to stop, or ``stop()``.
@MainActor
internal final class TerminalFling: NSObject {
    /// Moves the terminal by the points given; false once there is nothing left to move.
    internal typealias Step = @MainActor (CGFloat) -> Bool

    private var link: CADisplayLink?
    private var decay = FlingDecay(velocity: 0)
    private var started: CFTimeInterval = 0
    private var travelled: CGFloat = 0
    private var step: Step?

    /// Whether a fling is running.
    internal var isRunning: Bool {
        link != nil
    }

    /// Flings at `velocity` points a second, moving by `step`; a let-go slower than ``FlingDecay/minimumSpeed``
    /// doesn't fling.
    internal func start(velocity: CGFloat, step: @escaping Step) {
        stop()
        guard abs(velocity) > FlingDecay.minimumSpeed else {
            return
        }
        decay = FlingDecay(velocity: velocity)
        started = CACurrentMediaTime()
        travelled = 0
        self.step = step
        let made = CADisplayLink(target: self, selector: #selector(tick))
        made.add(to: .main, forMode: .common)
        link = made
    }

    /// Stops the fling where it is.
    internal func stop() {
        link?.invalidate()
        link = nil
        step = nil
    }

    @objc
    private func tick(_ link: CADisplayLink) {
        let elapsed = link.targetTimestamp - started
        let offset = decay.offset(after: elapsed)
        let moved = step?(offset - travelled) ?? false
        travelled = offset
        if !moved || decay.isOver(after: elapsed) {
            stop()
        }
    }

    deinit {
        // the display link holds the fling while it runs, and stop() lets it go
    }
}
