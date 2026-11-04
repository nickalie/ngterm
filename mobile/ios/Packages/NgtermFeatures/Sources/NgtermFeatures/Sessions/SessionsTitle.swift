import NgtermDesign
import NgtermPlatform
import SwiftUI

/// A machine's sessions screen's title: the machine's name over its mono address, read by VoiceOver as one heading
/// with the address in words ("build-01, nick at build-01.lan, port 22").
internal struct SessionsTitle: View {
    internal let machine: StoredMachine

    internal var body: some View {
        VStack(spacing: 0) {
            Text(machine.title)
                .font(.headline)
                .lineLimit(1)
            Text(machine.address)
                .font(Typography.address)
                .foregroundStyle(Palette.secondaryText.color)
                .lineLimit(1)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(String(localized: "\(machine.title), \(machine.spokenAddress)"))
        .accessibilityAddTraits(.isHeader)
    }
}
