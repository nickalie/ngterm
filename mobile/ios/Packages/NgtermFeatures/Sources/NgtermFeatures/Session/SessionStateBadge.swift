import NgtermDesign
import SwiftUI

/// An attached session's state as its toolbar shows it (mobile/android's `StateBadge`): the agent's status while the
/// stream is live, the stream's own state while it isn't.
internal struct SessionStateBadge: View {
    internal let session: AttachedSession

    internal var body: some View {
        switch session.live.connection {
        case .connected:
            if let status = session.status {
                AgentStatusBadge(status: status.text, colorHex: status.color, shape: status.shape)
            }

        case .connecting:
            if case .reconnecting = session.link {
                StatusBadge(.connecting, label: String(localized: "Reconnecting"))
            } else {
                StatusBadge(.connecting)
            }

        case .failed:
            StatusBadge(.connecting, label: String(localized: "Reconnecting"))

        case .unsupported:
            StatusBadge(.failed, label: String(localized: "Can't stream"))
        }
    }
}
