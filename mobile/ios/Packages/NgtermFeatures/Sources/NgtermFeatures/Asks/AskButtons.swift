import NgtermCore
import NgtermDesign
import SwiftUI

/// An ask's buttons in the origin's order, one per line and full width (mobile/android's `AskButton`).
///
/// The default is filled (``View/prominentAction()``, in `failed` when it is also the destructive one), the cancel
/// button quiet (the tint on the ground), the rest outlined; a destructive one that isn't the default says so in
/// `failed`. Every one is at least 44 pt tall.
internal struct AskButtons: View {
    // the outline's width, half of it inside the button's edge, and its corners' radius (half the least height), in
    // points
    private static let outline: Double = 1
    private static let outlineInset: Double = 0.5
    private static let outlineCorner: Double = 22

    internal let ask: RemoteAsk
    internal let onPress: @MainActor (String) -> Void

    internal var body: some View {
        VStack(spacing: Spacing.small) {
            ForEach(ask.buttons, id: \.id) { button in
                self.button(button)
                    .accessibilityIdentifier("ask-button-\(button.id)")
            }
        }
    }

    @ViewBuilder
    private func button(_ button: RemoteAskButton) -> some View {
        let isDestructive = button.id == ask.destructiveId
        let press = { onPress(button.id) }
        if button.id == ask.defaultId {
            Button(action: press) {
                Text(verbatim: button.label)
                    .frame(maxWidth: .infinity)
            }
            .prominentAction()
            .tint(isDestructive ? Palette.failed.color : Palette.tint.color)
        } else if button.id == ask.cancelId {
            Button(action: press) {
                label(button, color: isDestructive ? Palette.failed.color : Palette.tint.color)
            }
            .buttonStyle(.borderless)
        } else {
            let color = isDestructive ? Palette.failed.color : Palette.tint.color
            Button(action: press) {
                label(button, color: color)
                    .overlay {
                        // a capsule as tall as the button: a Capsule's own path draws a notch at its ends
                        RoundedRectangle(cornerRadius: Self.outlineCorner, style: .circular)
                            .inset(by: Self.outlineInset)
                            .stroke(color, lineWidth: Self.outline)
                    }
            }
            .buttonStyle(.borderless)
        }
    }

    private func label(_ button: RemoteAskButton, color: Color) -> some View {
        Text(verbatim: button.label)
            .foregroundStyle(color)
            .frame(maxWidth: .infinity, minHeight: Spacing.hitTarget)
            .contentShape(Rectangle())
    }
}
