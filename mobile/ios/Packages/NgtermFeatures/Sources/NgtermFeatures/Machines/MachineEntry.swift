import Foundation
import NgtermDesign
import SwiftUI

/// One machine's row: its link as the badge, what the link calls for, and its sessions' statuses.
///
/// The statuses show while it is connected. A reconnect counts down each second.
internal struct MachineEntry: View {
    @Environment(\.fixedNow)
    private var fixedNow

    internal let item: MachineItem
    internal let onConnect: @MainActor () -> Void
    internal let onReview: @MainActor () -> Void

    internal var body: some View {
        if let fixedNow {
            row(at: fixedNow)
        } else if case .reconnecting = item.link {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                row(at: context.date)
            }
        } else {
            row(at: .now)
        }
    }

    private func row(at now: Date) -> some View {
        let look = LinkLook(item.link, now: now)
        return MachineRow(
            name: item.machine.title,
            address: item.machine.address,
            tone: look.tone,
            toneLabel: look.label,
            detail: look.detail,
            action: look.action.map(rowAction)
        ) {
            if item.link == .connected, item.isListed {
                SessionsSummary(sessions: item.sessions)
            }
        }
        .accessibilityIdentifier("machine-row-\(item.machine.title)")
    }

    private func rowAction(_ action: LinkLook.Action) -> RowAction {
        switch action {
        case .connect(let title):
            RowAction(title, perform: onConnect)

        case .review:
            RowAction(String(localized: "Review"), perform: onReview)
        }
    }
}
