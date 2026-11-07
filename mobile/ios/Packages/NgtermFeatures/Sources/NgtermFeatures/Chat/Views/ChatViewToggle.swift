import SwiftUI

/// A session's Chat view (mobile/android's `ChatViewItem`), in every session's menu: the session's More, a machine's
/// list.
///
/// A toggle, checked while its panes running an agent show as a chat; a tap sets the other for that session alone.
/// VoiceOver reads it as a toggle.
internal struct ChatViewToggle: View {
    internal let choice: ChatViewChoice

    internal var body: some View {
        Toggle(
            String(localized: "Chat view"),
            systemImage: ChatStyle.chatSymbol,
            isOn: Binding {
                choice.isOn
            } set: { isOn in
                choice.set(isOn)
            }
        )
        .accessibilityIdentifier("session-chat-view")
    }
}
