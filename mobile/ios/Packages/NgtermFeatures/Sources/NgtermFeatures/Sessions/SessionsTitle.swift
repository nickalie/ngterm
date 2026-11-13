import NgtermDesign
import NgtermPlatform
import SwiftUI

/// A machine's sessions screen's title: the machine's name over its mono address, read by VoiceOver as one heading
/// with the address in words ("build-01, nick at build-01.lan, port 22"), and the app the machine runs after the
/// address when that isn't ngterm (a ``PeerBadge``: "build-01, …, port 22, agterm").
internal struct SessionsTitle: View {
    internal let machine: StoredMachine
    internal var peer: MachinePeer?

    private var badge: String? {
        peer.flatMap { $0.isForeign ? $0.badge : nil }
    }

    internal var body: some View {
        VStack(spacing: 0) {
            Text(machine.title)
                .font(.headline)
                .lineLimit(1)
            HStack(spacing: Spacing.xSmall) {
                Text(machine.address)
                    .font(Typography.address)
                    .foregroundStyle(Palette.secondaryText.color)
                    .lineLimit(1)
                if let badge {
                    PeerBadge(badge, note: peer?.note)
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        .accessibilityAddTraits(.isHeader)
    }

    private var label: String {
        let spoken = String(localized: "\(machine.title), \(machine.spokenAddress)")
        guard let badge else {
            return spoken
        }
        return String(localized: "\(spoken), \(badge)")
    }
}
