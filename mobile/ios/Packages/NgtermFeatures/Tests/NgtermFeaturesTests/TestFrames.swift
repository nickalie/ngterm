@testable import NgtermFeatures

/// A pane's frames as a terminal view takes them: what it was told, in order.
@MainActor
internal final class TestFrames: PaneFrames {
    /// What it was told: "dirty" and "attached".
    internal private(set) var seen: [String] = []

    /// Notes a new frame.
    internal func dirty() {
        seen.append("dirty")
    }

    /// Notes a fresh terminal.
    internal func attached() {
        seen.append("attached")
    }

    deinit {
        // nothing to release
    }
}
