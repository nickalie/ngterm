import NgtermDesign
import SwiftUI

/// Under a key's public half: once it is on the machines, point them all at it (mobile/android's `SwitchMachines`).
///
/// Once every machine uses it, it says so; with no machine at all, it shows nothing.
internal struct SwitchMachines: View {
    internal let item: KeyItem
    internal let onUse: @MainActor () -> Void

    internal var body: some View {
        if item.isEveryMachine, !item.usedBy.isEmpty {
            Text("Every machine signs in with this key.")
                .foregroundStyle(Palette.secondaryText.color)
                .accessibilityIdentifier("key-everywhere")
        } else if !item.isEveryMachine {
            VStack(alignment: .leading, spacing: Spacing.small) {
                Text("Then switch your machines")
                    .font(.headline)
                    .accessibilityAddTraits(.isHeader)
                Text(
                    """
                    Once this key is in authorized_keys on every machine, use it for all of them. They sign in \
                    with it from their next connection. Remove the old key from the machines only after that.
                    """
                )
                .foregroundStyle(Palette.secondaryText.color)
                .fixedSize(horizontal: false, vertical: true)
                Button {
                    onUse()
                } label: {
                    Text("Use for every machine")
                        .frame(maxWidth: .infinity)
                }
                .prominentAction()
                .accessibilityIdentifier("use-key-everywhere")
            }
            .padding(.top, Spacing.small)
        }
    }
}
