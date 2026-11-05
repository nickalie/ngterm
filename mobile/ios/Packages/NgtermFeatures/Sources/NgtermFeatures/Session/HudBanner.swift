import NgtermCore
import NgtermDesign
import SwiftUI

/// The origin's HUD over a session, as a strip under the toolbar (mobile/android's `HudBanner`): its message, its
/// detail under it, and a spinner when the origin runs one.
///
/// The origin's colors, size and position are its desktop's: here the strip sits where it covers no text, in the
/// app's colors. VoiceOver hears it as it changes.
internal struct HudBanner: View {
    internal let hud: RemoteHud

    internal var body: some View {
        HStack(spacing: Spacing.medium) {
            if hud.spinner != nil {
                ProgressView()
                    .controlSize(.small)
            }
            VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                Text(hud.message)
                    .font(Typography.banner)
                    .fixedSize(horizontal: false, vertical: true)
                if let detail = hud.detail, !detail.allSatisfy(\.isWhitespace) {
                    Text(detail)
                        .font(Typography.note)
                        .foregroundStyle(Palette.secondaryText.color)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, Spacing.medium)
        .padding(.vertical, Spacing.small)
        .frame(minHeight: Spacing.hitTarget)
        .background(
            Color(uiColor: .secondarySystemBackground),
            in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous)
        )
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("session-hud")
        .onChange(of: hud.message, initial: true) {
            AccessibilityNotification.Announcement(hud.message).post()
        }
    }
}
