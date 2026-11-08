import NgtermCoreFakes
import NgtermTerminal
import SwiftUI
import Testing
import UIKit

/// A pane's frame events through its feed to the view SwiftUI makes for it.
@MainActor
@Suite("the terminal feed")
internal struct TerminalFeedTests {
    // the terminal views under `view`
    private static func terminals(in view: UIView) -> [TerminalView] {
        ((view as? TerminalView).map { [$0] } ?? []) + view.subviews.flatMap(terminals)
    }

    @Test("a feed tells the view SwiftUI made of a new frame and of a fresh terminal; with no view yet, no one")
    internal func feed() throws {
        let pane = FakePane { since in RowBufferWriter(columns: 8, rows: 2, seq: since + 1).text(0, "feed").bytes() }
        let feed = TerminalFeed()
        // nothing shows the pane yet: nothing to tell, and nothing lost, since a new view reads from scratch
        feed.dirty()
        feed.attached()
        let terminal = TerminalViewRepresentable(
            pane: pane,
            feed: feed,
            palette: .copper,
            textSize: TerminalRig.textSize
        )
        let host = UIHostingController(rootView: terminal)
        let window = try TestWindow.make(CGRect(x: 0, y: 0, width: 390, height: 200))
        window.rootViewController = host
        window.isHidden = false
        host.view.layoutIfNeeded()
        let view = try #require(Self.terminals(in: host.view).first)
        view.drawFrame()
        #expect(!view.framePending)
        #expect(view.screen.text(0) == "feed")
        feed.dirty()
        #expect(view.framePending)
        view.drawFrame()
        feed.attached()
        #expect(view.framePending)
        view.drawFrame()
        #expect(pane.asked.suffix(3) == [0, 1, 0], "whole, then since the frame held, then whole again")
        window.isHidden = true
    }
}
