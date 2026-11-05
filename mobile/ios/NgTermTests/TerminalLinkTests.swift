import NgtermCoreFakes
import NgtermDesign
import NgtermTerminal
import Testing
import UIKit

/// The terminal's display link, in a window of the app's scene (a display link needs the app): it runs while a frame
/// waits, pauses once it has read it, and stops when the view leaves its window.
@MainActor
@Suite("the terminal's display link")
internal struct TerminalLinkTests {
    private static let patience: Duration = .seconds(2)
    private static let poll: Duration = .milliseconds(10)

    // waits for `done`, for at most `patience`; whether it came
    private static func wait(for done: () -> Bool) async throws -> Bool {
        let deadline = ContinuousClock.now + patience
        while !done(), ContinuousClock.now < deadline {
            try await Task.sleep(for: poll)
        }
        return done()
    }

    @Test("it reads a frame while one waits and then pauses, and stops when the view leaves its window")
    internal func runsWhileAFrameWaits() async throws {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let window = UIWindow(windowScene: try #require(scenes.first))
        let pane = FakePane { since in RowBufferWriter(columns: 8, rows: 2, seq: since + 1).text(0, "ok").bytes() }
        let view = TerminalView(pane: pane, palette: .copper, textSize: 13)
        view.frame = CGRect(x: 0, y: 0, width: 200, height: 100)
        #expect(!view.readsFrames)

        window.addSubview(view)
        #expect(view.readsFrames)
        #expect(try await Self.wait { !view.readsFrames })
        #expect(pane.asked == [0])
        #expect(view.screen.text(0) == "ok")

        view.dirty()
        #expect(view.readsFrames)
        #expect(try await Self.wait { !view.readsFrames })
        #expect(pane.asked == [0, 1])

        view.removeFromSuperview()
        view.dirty()
        #expect(!view.readsFrames)
    }
}
