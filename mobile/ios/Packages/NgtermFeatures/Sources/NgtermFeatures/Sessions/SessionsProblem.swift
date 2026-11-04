import Foundation
import NgtermDesign
import SwiftUI

/// What a machine's sessions screen shows when its link or its listing is in the way: what happened, and the one
/// action that moves on (Connect, Retry, Retry now, Review, Open the README).
///
/// A reconnect counts down each second to its retry.
internal struct SessionsProblem: View {
    /// What the problem's actions do.
    internal struct Actions {
        internal let connect: @MainActor () -> Void
        internal let refresh: @MainActor () -> Void
        internal let review: @MainActor () -> Void
        internal let openReadme: @MainActor () -> Void
    }

    @Environment(\.fixedNow)
    private var fixedNow

    internal let machine: String
    internal let problem: SessionsBody
    internal let actions: Actions

    internal var body: some View {
        switch problem {
        case .notConnected(let isDeviceOffline):
            notConnected(isDeviceOffline: isDeviceOffline)

        case .reconnecting(let retryAt):
            if let fixedNow {
                reconnecting(retryAt: retryAt, now: fixedNow)
            } else {
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    reconnecting(retryAt: retryAt, now: context.date)
                }
            }

        case .unreachable(let reason):
            StatePanel.error(
                String(localized: "Can't reach \(machine)"),
                message: reason,
                action: RowAction(String(localized: "Retry"), perform: actions.connect)
            )

        case .hostKeyChanged:
            StatePanel.error(
                String(localized: "\(machine)'s host key changed"),
                message: String(localized: "Nothing connects to it until you review the change."),
                action: RowAction(String(localized: "Review"), perform: actions.review)
            )

        case .missingCli:
            missingCli

        case .listFailed(let reason):
            StatePanel.error(
                String(localized: "Can't list the sessions on \(machine)"),
                message: reason,
                action: RowAction(String(localized: "Retry"), perform: actions.refresh)
            )

        case .connecting, .empty, .reading, .sessions:
            // the waiting states, the empty state and the list are the screen's own
            EmptyView()
        }
    }

    private var missingCli: some View {
        StatePanel.error(
            String(localized: "ngtermctl isn't on \(machine)"),
            message: String(
                localized: """
                    Install ngterm there and make sure ngtermctl is on the PATH of ssh logins (~/.local/bin or \
                    /usr/local/bin). The README's “A server without a display” shows how.
                    """
            ),
            action: RowAction(String(localized: "Open the README"), perform: actions.openReadme),
            secondaryAction: RowAction(String(localized: "Retry"), perform: actions.refresh)
        )
    }

    @ViewBuilder
    private func notConnected(isDeviceOffline: Bool) -> some View {
        if isDeviceOffline {
            StatePanel(
                title: String(localized: "You're offline"),
                systemImage: "wifi.slash",
                imageColor: Palette.offline,
                message: String(localized: "\(machine)'s sessions show here once the device is back on a network."),
                action: RowAction(String(localized: "Retry"), perform: actions.connect)
            )
        } else {
            StatePanel(
                title: String(localized: "\(machine) isn't connected"),
                systemImage: "bolt.horizontal.circle",
                imageColor: Palette.offline,
                message: String(localized: "Connect to see its sessions and start new ones."),
                action: RowAction(String(localized: "Connect"), perform: actions.connect)
            )
        }
    }

    private func reconnecting(retryAt: Date, now: Date) -> some View {
        let seconds = Int(retryAt.timeIntervalSince(now).rounded(.up))
        return StatePanel(
            title: String(localized: "The connection to \(machine) dropped"),
            systemImage: "arrow.clockwise",
            imageColor: Palette.connecting,
            message: seconds > 0
                ? String(localized: "Trying again in \(seconds) s.") : String(localized: "Trying again now."),
            action: RowAction(String(localized: "Retry now"), perform: actions.connect)
        )
    }
}
