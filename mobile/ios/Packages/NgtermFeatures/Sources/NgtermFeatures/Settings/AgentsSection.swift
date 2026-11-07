import SwiftUI

/// The agents (mobile/android's `AgentsSection`): Agent chat view, a switch, on by default.
internal struct AgentsSection: View {
    internal let model: SettingsModel

    internal var body: some View {
        Section {
            Toggle(isOn: model.binding(\.agentChat)) {
                ChoiceLabel(
                    String(localized: "Agent chat view"),
                    note: String(
                        localized: """
                            Show a pane running Claude Code or Codex as a chat. A session's More menu sets its own, \
                            and its toolbar switches a pane to the terminal and back.
                            """
                    )
                )
            }
            .accessibilityIdentifier("agent-chat")
        } header: {
            FormHint(String(localized: "Agents"), .header)
        }
    }
}
