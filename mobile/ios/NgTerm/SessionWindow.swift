import NgtermDesign
import NgtermFeatures
import SwiftUI

/// A session's own window (iPad): it holds the session for as long as it is open, so no other window shows it.
///
/// It shows the session view; once the session is let go (Detach, End session, a disconnect), the window closes. The
/// keyboard's moves to another session bring that session's own window forward, or open one.
internal struct SessionWindow: View {
    @Environment(\.dismissWindow)
    private var dismissWindow
    @Environment(\.openWindow)
    private var openWindow

    private let graph: AppGraph
    private let session: SessionWindowID
    private let windows: SessionWindows

    internal var body: some View {
        NavigationStack {
            SessionScreen(
                graph: graph,
                session: session.key,
                actions: SessionActions(
                    close: {
                        dismissWindow()
                    },
                    open: { key in
                        openWindow(value: SessionWindowID(key))
                    },
                    openInNewWindow: nil
                )
            )
        }
        .ngtermTheme()
        .onAppear {
            windows.opened(session)
        }
        .onDisappear {
            windows.closed(session)
        }
    }

    internal init(graph: AppGraph, session: SessionWindowID, windows: SessionWindows) {
        self.graph = graph
        self.session = session
        self.windows = windows
    }
}
