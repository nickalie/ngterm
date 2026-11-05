import NgtermTerminal
import Testing

/// How a fling slows down: as a list does, coming to rest a finite way off.
@Suite("a fling's decay")
internal struct FlingDecayTests {
    @Test("it starts at its speed, slows every moment, and goes a finite way in its direction")
    internal func slows() {
        let decay = FlingDecay(velocity: 2_000)
        #expect(decay.offset(after: 0) == 0)
        #expect(decay.speed(after: 0) == 2_000)
        #expect(decay.speed(after: 0.5) < decay.speed(after: 0.1))
        #expect(decay.offset(after: 0.1) > 0)
        #expect(decay.offset(after: 0.5) > decay.offset(after: 0.1))
        // the whole way is finite: about half a second of the start's speed at the normal rate (1 / (−1000 ln 0.998))
        let whole = decay.offset(after: 60)
        #expect(abs(decay.offset(after: 120) - whole) < 0.001)
        #expect(whole > 990 && whole < 1_010)
        #expect(!decay.isOver(after: 0.5))
        #expect(decay.isOver(after: 3))
    }

    @Test("an upward fling goes up")
    internal func upward() {
        let decay = FlingDecay(velocity: -800)
        #expect(decay.offset(after: 0.2) < 0)
        #expect(decay.speed(after: 0.2) < 0)
    }
}
