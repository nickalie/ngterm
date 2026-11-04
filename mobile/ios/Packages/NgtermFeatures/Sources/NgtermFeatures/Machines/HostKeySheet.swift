import NgtermDesign
import NgtermPlatform
import SwiftUI

/// The first connection to `machine`, which offered `fingerprint` and waits for the user.
///
/// Trust and connect pins it and connects; Don't connect, or swiping the sheet away, refuses it.
internal struct HostKeySheet: View {
    /// What prints a machine's host key fingerprints, whatever kinds of key it has.
    internal static let fingerprintCommand = "ssh-keyscan localhost 2>/dev/null | ssh-keygen -lf -"

    internal let machine: StoredMachine
    internal let fingerprint: String
    internal let onAnswer: @MainActor (Bool) -> Void

    internal var body: some View {
        NavigationStack {
            PromptPage(
                tone: .attention,
                title: String(localized: "Trust \(machine.title)?"),
                machine: machine,
                message: String(
                    localized: """
                        This is the first connection to \(machine.title). Compare its host key's fingerprint \
                        with the one the machine shows before you trust it.
                        """
                )
            ) {
                ValuePlate(String(localized: "Fingerprint"), value: fingerprint)
                ValuePlate(String(localized: "On the machine, run"), value: Self.fingerprintCommand) {
                    CopyButton(Self.fingerprintCommand)
                }
                VStack(spacing: Spacing.small) {
                    Button {
                        onAnswer(true)
                    } label: {
                        Text("Trust and connect")
                            .frame(maxWidth: .infinity)
                    }
                    .prominentAction()
                    .accessibilityIdentifier("trust-host-key")
                    Button {
                        onAnswer(false)
                    } label: {
                        Text("Don't connect")
                            .frame(maxWidth: .infinity, minHeight: Spacing.hitTarget)
                    }
                    .accessibilityIdentifier("decline-host-key")
                }
                .padding(.top, Spacing.small)
            }
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}
