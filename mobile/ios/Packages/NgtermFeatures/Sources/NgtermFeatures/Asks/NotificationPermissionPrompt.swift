import NgtermDesign
import NgtermPlatform
import SwiftUI

/// The permission to notify, asked once and with its reason (mobile/android's `NotificationPermissionPrompt`), at the
/// moment it starts to matter: as the first session is attached, so its questions can reach the user out of sight.
///
/// It asks only while the user hasn't answered the system's question. **Turn on notifications** raises the system's
/// question; **Not now**, or a swipe, leaves it for Settings. Either way it isn't asked again: it is marked asked as
/// it shows, so a second window doesn't ask too.
internal struct NotificationPermissionPrompt: ViewModifier {
    internal let graph: AppGraph
    @State private var isAsking = false

    private var hasAttached: Bool {
        graph.connections.attached.values.contains { !$0.isEmpty }
    }

    internal func body(content: Content) -> some View {
        content
            .onChange(of: hasAttached, initial: true) { _, attached in
                if attached {
                    Task {
                        await consider()
                    }
                }
            }
            .sheet(isPresented: $isAsking) {
                NotificationPermissionQuestion {
                    isAsking = false
                    Task {
                        _ = await graph.notices.requestPermission()
                    }
                } onLater: {
                    isAsking = false
                }
                .ngtermTheme()
            }
    }

    private func consider() async {
        let prefs = graph.notificationPrefs
        guard !prefs.prefs.isPermissionAsked else {
            return
        }
        // a choice the store can't keep asks again at the next launch
        try? prefs.markPermissionAsked()
        if await graph.notices.permission() == .undecided {
            isAsking = true
        }
    }
}

extension View {
    /// Asks once, as the first session of `graph` is attached, whether the app may notify.
    public func notificationPermissionPrompt(_ graph: AppGraph) -> some View {
        modifier(NotificationPermissionPrompt(graph: graph))
    }
}
