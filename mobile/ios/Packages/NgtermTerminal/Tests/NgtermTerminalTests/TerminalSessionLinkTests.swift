import CoreGraphics
import NgtermCore
import NgtermCoreFakes
import NgtermTerminal
import Testing
import UIKit

/// A tap on a session link and the app's appearance, against the fake pane.
///
/// As mobile/android's `TerminalSessionLinkTest`: the link opens the session it names in place of the tap's own work,
/// any other tap does that work, and the terminal is given the app's appearance.
@MainActor
@Suite("session links and the appearance")
internal struct TerminalSessionLinkTests {
    nonisolated private static let columns = 40
    nonisolated private static let rows = 10
    private static let other = fakeSession(id: "00000000-0000-0000-0000-0000000000A4", name: "other")
    private static let halves: CGFloat = 2

    private let pane: FakePane
    private let view: TerminalView

    private var clicks: [PaneMouseAction] {
        pane.inputs.compactMap { input in
            if case .mouse(let mouse) = input {
                return mouse.action
            }
            return nil
        }
    }

    // row 0 links a listed session, row 1 a link the machine lists no session for, row 2 is plain text
    internal init() {
        let linked = FakePane { since in
            RowBufferWriter(columns: Self.columns, rows: Self.rows, seq: since + 1)
                .row(0, FakeRun(0, "other", attributes: FakeRun.hyperlink))
                .row(1, FakeRun(0, "gone", attributes: FakeRun.hyperlink))
                .text(2, "plain")
                .bytes()
        }
        linked.links = [CellPoint(x: 2, y: 0): Self.other]
        linked.tracking = true
        let made = TerminalView(
            pane: linked,
            palette: .copper,
            textSize: 13,
            keyboard: TerminalKeyboard(hardware: HardwareKeyboard(fixed: false))
        )
        made.frame = CGRect(x: 0, y: 0, width: 390, height: 200)
        made.layoutIfNeeded()
        made.drawFrame()
        pane = linked
        view = made
    }

    private func point(_ column: Int, _ row: Int) -> CGPoint {
        let font = view.font
        return CGPoint(
            x: font.left(column) + font.cellWidth / Self.halves,
            y: font.top(row) + font.cellHeight / Self.halves
        )
    }

    @Test("a tap on a session link opens its session and clicks nothing")
    internal func linkOpens() async {
        var opened: [RemoteSession] = []
        view.onSessionLink = { opened.append($0) }
        await view.gestures.tap(at: point(2, 0), time: 10)?.value
        #expect(opened == [Self.other])
        #expect(clicks.isEmpty)
    }

    @Test("a tap on a link naming no listed session, or on plain text, does the tap's own work")
    internal func otherTapsClick() async {
        var opened: [RemoteSession] = []
        view.onSessionLink = { opened.append($0) }
        await view.gestures.tap(at: point(1, 1), time: 10)?.value
        #expect(view.gestures.tap(at: point(1, 2), time: 20) == nil)
        #expect(opened.isEmpty)
        #expect(clicks == [.press, .release, .press, .release])
    }

    @Test("a terminal with nowhere to open a link clicks on it")
    internal func noHandlerClicks() {
        #expect(view.gestures.tap(at: point(2, 0), time: 10) == nil)
        #expect(clicks == [.press, .release])
    }

    @Test("the terminal is given the app's appearance")
    internal func appearance() {
        view.setAppearance(dark: true)
        view.setAppearance(dark: false)
        #expect(pane.colorSchemes == [.dark, .light])
    }
}
