import NgtermDesign
import SwiftUI

/// The main action of a step, full width, with a spinner in front of its words while it runs (and no second tap).
internal struct BusyButton: View {
    internal let title: String
    internal let isBusy: Bool
    internal let action: @MainActor () -> Void

    internal var body: some View {
        Button {
            if !isBusy {
                action()
            }
        } label: {
            HStack(spacing: Spacing.small) {
                if isBusy {
                    ProgressView()
                        .tint(Palette.onTint.color)
                }
                Text(title)
            }
            .frame(maxWidth: .infinity)
        }
        .prominentAction()
        .accessibilityValue(isBusy ? Text("In progress") : Text(verbatim: ""))
    }
}
