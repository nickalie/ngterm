import SwiftUI
import Testing

@testable import NgtermFeatures

/// A golden of the detail column when the session it would show is elsewhere: in a window of its own, with Show window,
/// and taken over by another main window, with Show here.
@MainActor
@Suite("session detail snapshots")
internal struct SessionDetailSnapshotTests {
    private static let docs = SessionWindowID(machineID: "build-01", sessionID: "A1")

    private static func detail(_ model: ScaffoldModel) -> some View {
        SessionDetail(model: model) { _ in
            Text(verbatim: "the session")
        }
        .frame(maxHeight: .infinity)
    }

    @Test("a session open in its own window, and one another window took over")
    internal func elsewhere() {
        let windows = SessionWindows()
        let own = ScaffoldModel(windows: windows)
        _ = own.open(Self.docs)
        windows.opened(Self.docs)
        #expect(own.shownSessionPlacement == .ownWindow)
        let api = SessionWindowID(machineID: "build-01", sessionID: "B2")
        let first = ScaffoldModel(windows: windows)
        _ = first.open(api)
        let second = ScaffoldModel(windows: windows)
        _ = second.open(api)
        // the window that took the session over holds it while it lives
        withExtendedLifetime(second) {
            #expect(first.shownSessionPlacement == .otherWindow)
            let view = VStack(spacing: 0) {
                Self.detail(own)
                Divider()
                Self.detail(first)
            }
            .background(Color(uiColor: .systemGroupedBackground))
            ScreenSnapshot.check(view, named: "session-elsewhere", variant: .light)
        }
    }
}
