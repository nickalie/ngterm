import NgtermDesign
import NgtermFeatures
import SwiftUI

/// A session's own window (iPad): it holds the session for as long as it is open, so no other window shows it.
internal struct SessionWindow: View {
    private let session: SessionWindowID
    private let windows: SessionWindows

    internal var body: some View {
        StatePanel.loading(String(localized: "Opening the session"))
            .ngtermTheme()
            .onAppear {
                windows.opened(session)
            }
            .onDisappear {
                windows.closed(session)
            }
    }

    internal init(session: SessionWindowID, windows: SessionWindows) {
        self.session = session
        self.windows = windows
    }
}
