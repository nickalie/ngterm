import NgtermDesign
import SwiftUI

/// An attached session under its machine in the sidebar (mobile/android's `SidebarSessions`): its name and its status
/// (the session's own view says its context), indented under the machine, the one this window shows marked with a
/// checkmark in the tint; a tap opens it.
internal struct SidebarSessionRow: View {
    internal let session: AttachedSession
    internal let isShown: Bool
    internal let onOpen: @MainActor () -> Void

    internal var body: some View {
        Button(action: onOpen) {
            label
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isShown ? .isSelected : [])
        .accessibilityIdentifier("sidebar-session-\(session.key.sessionId)")
    }

    private var label: some View {
        HStack(spacing: Spacing.small) {
            SessionRow(name: session.name, line: nil, isCompanion: true) {
                if let status = session.status {
                    AgentStatusBadge(status: status.text, colorHex: status.color, shape: status.shape)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if isShown {
                Image(systemName: "checkmark")
                    .font(Typography.statusWord)
                    .foregroundStyle(Palette.tint.color)
                    .accessibilityHidden(true)
            }
        }
        .contentShape(Rectangle())
    }
}
