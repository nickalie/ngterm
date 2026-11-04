import NgtermCore
import NgtermDesign
import SwiftUI

/// A connected machine's sessions at a glance.
///
/// A badge per status its agents report, with how many sessions have it, then how many are idle; "No sessions" for
/// none. At an accessibility text size the badges stack.
internal struct SessionsSummary: View {
    @Environment(\.dynamicTypeSize)
    private var typeSize

    internal let sessions: [RemoteSession]

    internal var body: some View {
        if sessions.isEmpty {
            Text("No sessions")
                .font(Typography.detail)
                .foregroundStyle(Palette.secondaryText.color)
        } else {
            let layout =
                typeSize.isAccessibilitySize
                ? AnyLayout(VStackLayout(alignment: .leading, spacing: Spacing.xSmall))
                : AnyLayout(HStackLayout(spacing: Spacing.medium))
            layout {
                ForEach(groups, id: \.status) { group in
                    AgentStatusBadge(
                        status: group.status,
                        colorHex: group.first.statusColor,
                        shape: group.first.statusShape,
                        label: "\(group.count) \(Self.word(for: group.status).lowercased())"
                    )
                }
                if idle > 0 {
                    StatusBadge(.idle, label: String(localized: "\(idle) idle"))
                }
            }
        }
    }

    // the sessions with a status, by status in the order they first appear
    private var groups: [StatusGroup] {
        var found: [StatusGroup] = []
        for session in sessions {
            guard let status = session.status?.trimmingCharacters(in: .whitespaces), !status.isEmpty else {
                continue
            }
            if let index = found.firstIndex(where: { $0.status == status }) {
                found[index].count += 1
            } else {
                found.append(StatusGroup(status: status, first: session))
            }
        }
        return found
    }

    private var idle: Int {
        sessions.count { $0.status?.trimmingCharacters(in: .whitespaces).isEmpty ?? true }
    }

    private static func word(for status: String) -> String {
        AgentState.fromWire(status)?.word ?? status
    }
}
