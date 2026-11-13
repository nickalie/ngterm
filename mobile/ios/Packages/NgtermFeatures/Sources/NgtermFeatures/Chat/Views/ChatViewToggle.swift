import SwiftUI

/// A session's Chat view (mobile/android's `ChatViewItem`), in every session's menu: the session's More, a machine's
/// list.
///
/// A toggle, checked while its panes running an agent show as a chat; a tap sets the other for that session alone.
/// On a machine whose app has no chat view (agterm) it is off and takes no tap, its subtitle saying why. VoiceOver
/// reads it as a toggle.
internal struct ChatViewToggle: View {
    internal let choice: ChatViewChoice

    internal var body: some View {
        if let app = choice.noChatOn {
            // the menu draws the second text as the item's subtitle
            Toggle(isOn: .constant(false)) {
                Label(String(localized: "Chat view"), systemImage: ChatStyle.chatSymbol)
                Text(String(localized: "\(app) has no chat view"))
            }
            .disabled(true)
            .accessibilityIdentifier("session-chat-view")
        } else {
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
}
