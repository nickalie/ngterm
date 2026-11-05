import SwiftUI

/// An attached session's More menu, in its toolbar (mobile/android's `SessionMenu`): Detach at once, Open in New
/// Window where the device has windows, and End session, which the view confirms first.
internal struct SessionMenu: View {
    internal let name: String
    internal let onDetach: @MainActor () -> Void
    internal let onNewWindow: (@MainActor () -> Void)?
    internal let onEnd: @MainActor () -> Void

    internal var body: some View {
        Menu {
            Button(String(localized: "Detach"), systemImage: "eject", action: onDetach)
                .accessibilityIdentifier("session-detach")
            if let onNewWindow {
                Button(
                    String(localized: "Open in New Window"),
                    systemImage: "macwindow.badge.plus",
                    action: onNewWindow
                )
                .accessibilityIdentifier("session-new-window")
            }
            Divider()
            Button(String(localized: "End session"), systemImage: "xmark.circle", role: .destructive, action: onEnd)
                .accessibilityIdentifier("session-end")
        } label: {
            Label(String(localized: "More for \(name)"), systemImage: "ellipsis")
        }
        .accessibilityIdentifier("session-menu")
    }
}
