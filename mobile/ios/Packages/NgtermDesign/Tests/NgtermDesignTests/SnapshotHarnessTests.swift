import SnapshotTesting
import SwiftUI
import Testing

/// The snapshot tests' harness: goldens are recorded on the simulator runtime build/pins.env pins, and verified on it,
/// on the phone and the tablet alike (a fixed layout at a fixed scale renders the same on both).
@MainActor
@Suite("snapshot harness")
internal struct SnapshotHarnessTests {
    @Test("a golden recorded on the pinned runtime verifies on it")
    internal func goldenVerifies() {
        let swatches = HStack(spacing: 0) {
            Color.red
            Color.green
            Color.blue
        }
        assertSnapshot(
            of: swatches,
            as: .image(layout: .fixed(width: 60, height: 20), traits: UITraitCollection(displayScale: 2))
        )
    }
}
