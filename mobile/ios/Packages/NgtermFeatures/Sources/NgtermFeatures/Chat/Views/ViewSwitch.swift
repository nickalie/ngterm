import SwiftUI

/// The focused pane's other view, in the session's toolbar (mobile/android's `ViewSwitch`), named for what a tap does:
/// "Show terminal" (the terminal's glyph) over a chat, "Show chat" (the chat's) over a terminal; only while the focused
/// pane runs an agent and its session's chat view is on.
internal struct ViewSwitch: View {
    internal let view: PaneView
    internal let onView: @MainActor (PaneView) -> Void

    internal var body: some View {
        let other: PaneView = view == .chat ? .terminal : .chat
        Button {
            onView(other)
        } label: {
            switch other {
            case .terminal:
                Label(String(localized: "Show terminal"), systemImage: "terminal")

            case .chat:
                Label(String(localized: "Show chat"), systemImage: ChatStyle.chatSymbol)
            }
        }
        .accessibilityIdentifier("session-view-switch")
    }
}
