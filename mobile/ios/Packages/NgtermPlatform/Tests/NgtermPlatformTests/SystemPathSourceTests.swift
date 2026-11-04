import NgtermPlatform
import Testing

/// The system's path source, on the device's own network: it reports a path each time it is started.
@MainActor
@Suite("the system's path source")
internal struct SystemPathSourceTests {
    private static let patience = Duration.seconds(10)
    private static let step = Duration.milliseconds(20)

    // waits until `condition` holds, or fails after ``patience``
    private static func wait(until condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + patience
        while !condition() {
            try #require(ContinuousClock.now < deadline, "no path reported")
            try await Task.sleep(for: step)
        }
    }

    @Test("a source stopped and started again reports again")
    internal func restart() async throws {
        let source = SystemPathSource()
        let paths = PathCount()
        defer { source.stop() }
        source.start { _ in paths.reported += 1 }
        try await Self.wait { paths.reported >= 1 }
        source.stop()
        let before = paths.reported
        source.start { _ in paths.reported += 1 }
        try await Self.wait { paths.reported > before }
    }
}
