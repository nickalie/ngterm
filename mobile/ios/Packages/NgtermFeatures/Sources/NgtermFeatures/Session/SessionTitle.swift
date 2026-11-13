import NgtermDesign
import SwiftUI

/// The session's title in its toolbar: its name over its state, the app its machine runs when that isn't ngterm (a
/// ``PeerBadge``) and its context, read by VoiceOver as one heading.
internal struct SessionTitle: View {
    internal let session: AttachedSession

    internal var body: some View {
        VStack(spacing: Spacing.xxSmall) {
            Text(session.name)
                .font(.headline)
                .lineLimit(1)
            HStack(spacing: Spacing.small) {
                SessionStateBadge(session: session)
                if let peer = session.peer, peer.isForeign {
                    PeerBadge(peer.badge, note: peer.note)
                }
                if let context = session.context {
                    Text(context)
                        .font(Typography.note)
                        .foregroundStyle(Palette.secondaryText.color)
                        .lineLimit(1)
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
        .accessibilityIdentifier("session-title")
    }
}
