import NgtermDesign
import SwiftUI

/// A card's options, one per line in their order (mobile/android's `CardButtons`): the first filled, the rest outlined;
/// none while one is on its way (`isBusy`, which says Sending under them) or until the card can be answered
/// (`isEnabled`).
internal struct CardButtons: View {
    private static let outline: Double = 1
    private static let outlineInset: Double = 0.5
    private static let outlineCorner: Double = 22

    internal let labels: [String]
    internal let isBusy: Bool
    internal var isEnabled = true
    internal let onPress: @MainActor (String) -> Void

    internal var body: some View {
        let isActive = isEnabled && !isBusy
        VStack(spacing: Spacing.small) {
            ForEach(Array(labels.enumerated()), id: \.offset) { index, label in
                if index == 0 {
                    Button {
                        onPress(label)
                    } label: {
                        Text(verbatim: label)
                            .frame(maxWidth: .infinity)
                    }
                    .prominentAction()
                    .disabled(!isActive)
                } else {
                    Button {
                        onPress(label)
                    } label: {
                        Text(verbatim: label)
                            .foregroundStyle(isActive ? Palette.tint.color : Palette.secondaryText.color)
                            .frame(maxWidth: .infinity, minHeight: Spacing.hitTarget)
                            .contentShape(Rectangle())
                            .overlay {
                                RoundedRectangle(cornerRadius: Self.outlineCorner, style: .circular)
                                    .inset(by: Self.outlineInset)
                                    .stroke(
                                        isActive ? Palette.tint.color : Palette.secondaryText.color,
                                        lineWidth: Self.outline
                                    )
                            }
                    }
                    .buttonStyle(.borderless)
                    .disabled(!isActive)
                }
            }
            if isBusy {
                ChatSending()
            }
        }
    }
}
