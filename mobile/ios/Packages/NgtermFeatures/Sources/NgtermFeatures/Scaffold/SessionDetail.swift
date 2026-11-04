import NgtermDesign
import SwiftUI

/// The detail column: the session this window shows, or why it shows none.
///
/// Nothing is picked yet, or the session has a window of its own, which this brings forward.
internal struct SessionDetail<Session: View>: View {
    @Environment(\.openWindow)
    private var openWindow

    internal let model: ScaffoldModel
    internal let session: (SessionWindowID) -> Session

    internal var body: some View {
        if let shown = model.session {
            if model.shownSessionPlacement == .ownWindow {
                StatePanel(
                    title: String(localized: "Open in its own window"),
                    systemImage: "macwindow",
                    message: String(localized: "This session has a window of its own, so it shows there and not here."),
                    action: RowAction(String(localized: "Show window")) {
                        openWindow(value: shown)
                    }
                )
            } else {
                session(shown)
            }
        } else {
            StatePanel(
                title: String(localized: "No session open"),
                systemImage: "terminal",
                message: String(localized: "Pick a session to see its panes.")
            )
        }
    }
}
