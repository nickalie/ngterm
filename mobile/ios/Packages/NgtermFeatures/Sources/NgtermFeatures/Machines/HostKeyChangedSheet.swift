import NgtermDesign
import SwiftUI

/// The hard stop when a machine offered another key than its pinned one.
///
/// It says what that can mean, shows both fingerprints, and offers two ways on. Back to machines is the safe one and
/// the main action; Remove pinned key is the deliberate one, behind a confirmation, and connects again, which asks
/// about the key on offer.
internal struct HostKeyChangedSheet: View {
    @Environment(\.dismiss)
    private var dismiss

    internal let model: MachinesModel
    internal let item: MachineItem
    @State private var isConfirming = false

    internal var body: some View {
        let title = item.machine.title
        NavigationStack {
            page(title: title)
                .navigationBarTitleDisplayMode(.inline)
                .confirmingDestructive(
                    String(localized: "Remove the pinned key of \(title)?"),
                    isPresented: $isConfirming,
                    message: String(
                        localized: """
                            The next connection asks you to trust whatever key \(title) offers then. \
                            Only do this if you know why its key changed.
                            """
                    ),
                    action: RowAction(String(localized: "Remove pinned key")) {
                        model.removePin(machineId: item.id)
                        dismiss()
                    }
                )
        }
    }

    private var actions: some View {
        VStack(spacing: Spacing.small) {
            Button {
                dismiss()
            } label: {
                Text("Back to machines")
                    .frame(maxWidth: .infinity)
            }
            .prominentAction()
            Button(role: .destructive) {
                isConfirming = true
            } label: {
                Text("Remove pinned key")
                    .frame(maxWidth: .infinity, minHeight: Spacing.hitTarget)
            }
            .accessibilityIdentifier("remove-pin")
        }
        .padding(.top, Spacing.small)
    }

    private func page(title: String) -> some View {
        PromptPage(
            tone: .failed,
            title: String(localized: "\(title)'s host key changed"),
            machine: item.machine,
            message: String(
                localized: """
                    \(title) offered a different host key from the one you trusted. Someone may be intercepting \
                    the connection, or the machine was reinstalled or its keys were made anew. ngterm won't \
                    connect until you check.
                    """
            )
        ) {
            if case .hostKeyChanged(let pinned, let offered) = item.link {
                ValuePlate(String(localized: "Trusted until now"), value: pinned)
                ValuePlate(String(localized: "Offered now"), value: offered)
            }
            Text(
                """
                If you know why the key changed, remove the pinned key and confirm the new one when you \
                connect. If you don't, ask whoever runs the machine before you do anything.
                """
            )
            .font(.subheadline)
            .foregroundStyle(Palette.secondaryText.color)
            actions
        }
    }
}
